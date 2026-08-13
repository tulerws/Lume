use std::{collections::HashSet, path::Path};

use serde::Serialize;

use crate::domain::{
    AgentSession, SessionActivity, SessionResult, WorkflowConnectionDefinition,
    WorkflowContextPolicy, WorkflowContextSelection, WorkflowGroupDefinition, WorkflowRole,
    WorkflowStepDefinition,
};

const MAX_RESPONSE_CHARS: usize = 24_000;
const MAX_PLAN_CHARS: usize = 6_000;
const MAX_ACTIVITY_DETAIL_CHARS: usize = 700;
const MAX_DIFF_CHARS: usize = 1_600;
const MAX_FILES: usize = 32;
const MAX_CHECKS: usize = 16;
const MAX_ACTIVITIES: usize = 18;
#[cfg(test)]
const DEFAULT_MAX_CONTEXT_TOKENS: usize = 20_000;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowContextFile {
    pub path: String,
    pub external: bool,
    pub added: usize,
    pub removed: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowContextCheck {
    pub summary: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowContextActivity {
    pub kind: String,
    pub title: String,
    pub detail: Option<String>,
    pub status: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowContextRedaction {
    pub kind: String,
    pub summary: String,
    pub count: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowContextPackage {
    pub version: u8,
    pub workflow_id: String,
    pub source_step_id: String,
    pub target_step_id: String,
    pub source_result_id: String,
    pub policy: WorkflowContextPolicy,
    pub objective: String,
    pub source_role: String,
    pub target_role: String,
    pub result: Option<String>,
    pub files: Vec<WorkflowContextFile>,
    pub checks: Vec<WorkflowContextCheck>,
    pub plan: Option<String>,
    pub relevant_activity: Vec<WorkflowContextActivity>,
    pub next_instruction: String,
    pub redactions: Vec<WorkflowContextRedaction>,
    pub estimated_tokens: usize,
    pub markdown: String,
}

#[derive(Default)]
struct RedactionCounts {
    secrets: usize,
    sensitive_files: usize,
    invalid_file_references: usize,
    patch_markers: usize,
    truncated_sections: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PathRejection {
    Invalid,
    Sensitive,
}

pub fn effective_selection(connection: &WorkflowConnectionDefinition) -> WorkflowContextSelection {
    match connection.context_policy {
        WorkflowContextPolicy::Minimal => WorkflowContextSelection {
            response: true,
            files: false,
            checks: false,
            plan: false,
            activity: false,
            diffs: false,
        },
        WorkflowContextPolicy::Standard => WorkflowContextSelection::default(),
        WorkflowContextPolicy::Detailed => WorkflowContextSelection {
            response: true,
            files: true,
            checks: true,
            plan: true,
            activity: true,
            diffs: true,
        },
        WorkflowContextPolicy::Custom => connection.context_selection.clone(),
    }
}

#[cfg(test)]
pub fn build_context_package(
    group: &WorkflowGroupDefinition,
    connection_id: &str,
    objective: &str,
    source_result_id: Option<&str>,
    sessions: &[AgentSession],
) -> Result<WorkflowContextPackage, String> {
    build_context_package_with_limit(
        group,
        connection_id,
        objective,
        source_result_id,
        sessions,
        DEFAULT_MAX_CONTEXT_TOKENS,
    )
}

pub fn build_context_package_with_limit(
    group: &WorkflowGroupDefinition,
    connection_id: &str,
    objective: &str,
    source_result_id: Option<&str>,
    sessions: &[AgentSession],
    max_context_tokens: usize,
) -> Result<WorkflowContextPackage, String> {
    let objective = objective.trim();
    if objective.is_empty() {
        return Err("Add an objective before previewing this context".into());
    }
    let connection = group
        .connections
        .iter()
        .find(|connection| connection.id == connection_id)
        .ok_or_else(|| "Workflow connection not found".to_string())?;
    let source_step = workflow_step(group, &connection.from_step_id)?;
    let target_step = workflow_step(group, &connection.to_step_id)?;
    let (source_session, result) =
        select_source_result(sessions, &source_step.session_native_id, source_result_id)?;
    let turn_start = source_session
        .activities
        .iter()
        .filter(|activity| activity.kind == "prompt" && activity.created_at <= result.created_at)
        .map(|activity| activity.created_at)
        .max()
        .unwrap_or(i64::MIN);
    let turn_activities = source_session
        .activities
        .iter()
        .filter(|activity| {
            activity.created_at >= turn_start && activity.created_at <= result.created_at
        })
        .collect::<Vec<_>>();
    let selection = effective_selection(connection);
    let mut redactions = RedactionCounts::default();
    let result_text = selection
        .response
        .then(|| clean_text(&result.response, MAX_RESPONSE_CHARS, &mut redactions))
        .filter(|value| !value.is_empty());
    let files = if selection.files {
        collect_files(
            &source_session,
            &result,
            &turn_activities,
            selection.diffs,
            &mut redactions,
        )
    } else {
        Vec::new()
    };
    let checks = if selection.checks {
        collect_checks(&result, &turn_activities, &mut redactions)
    } else {
        Vec::new()
    };
    let plan = if selection.plan {
        turn_activities
            .iter()
            .rev()
            .find(|activity| matches!(activity.kind.as_str(), "plan" | "plan_document"))
            .and_then(|activity| activity.detail.as_deref())
            .map(|value| clean_text(value, MAX_PLAN_CHARS, &mut redactions))
            .filter(|value| !value.is_empty())
    } else {
        None
    };
    let relevant_activity = if selection.activity {
        collect_activity(&turn_activities, &mut redactions)
    } else {
        Vec::new()
    };
    let next_instruction = next_instruction(target_step, connection, &mut redactions);
    let objective = clean_text(objective, 4_000, &mut redactions);
    let redactions = redaction_summary(redactions);
    let mut package = WorkflowContextPackage {
        version: 1,
        workflow_id: group.id.clone(),
        source_step_id: source_step.id.clone(),
        target_step_id: target_step.id.clone(),
        source_result_id: result.id.clone(),
        policy: connection.context_policy,
        objective,
        source_role: role_label(source_step),
        target_role: role_label(target_step),
        result: result_text,
        files,
        checks,
        plan,
        relevant_activity,
        next_instruction,
        redactions,
        estimated_tokens: 0,
        markdown: String::new(),
    };
    refresh_rendered_package(&mut package);
    enforce_global_limit(&mut package, max_context_tokens.max(1_000));
    Ok(package)
}

fn refresh_rendered_package(package: &mut WorkflowContextPackage) {
    package.markdown = render_markdown(package);
    package.estimated_tokens = package.markdown.chars().count().div_ceil(4);
}

fn enforce_global_limit(package: &mut WorkflowContextPackage, max_tokens: usize) {
    if package.estimated_tokens <= max_tokens {
        return;
    }

    package.relevant_activity.clear();
    for file in &mut package.files {
        file.diff = None;
    }
    package.plan = None;
    while package.checks.len() > 1 {
        package.checks.pop();
    }
    while package.files.len() > 1 {
        package.files.pop();
    }
    refresh_rendered_package(package);

    if package.estimated_tokens > max_tokens {
        if let Some(response) = package.result.take() {
            refresh_rendered_package(package);
            let reserved = package.markdown.chars().count();
            let available = max_tokens.saturating_mul(4).saturating_sub(reserved + 80);
            if available > 0 {
                let mut shortened = response.chars().take(available).collect::<String>();
                if shortened.chars().count() < response.chars().count() {
                    shortened.push_str("\n… [truncated by Lume]");
                }
                package.result = Some(shortened);
            }
        }
    }

    if let Some(limit) = package
        .redactions
        .iter_mut()
        .find(|entry| entry.kind == "limit")
    {
        limit.count = limit.count.saturating_add(1);
    } else {
        package.redactions.push(WorkflowContextRedaction {
            kind: "limit".into(),
            summary: "Oversized context sections were truncated".into(),
            count: 1,
        });
    }
    refresh_rendered_package(package);
}

fn workflow_step<'a>(
    group: &'a WorkflowGroupDefinition,
    step_id: &str,
) -> Result<&'a WorkflowStepDefinition, String> {
    group
        .steps
        .iter()
        .find(|step| step.id == step_id)
        .ok_or_else(|| "Workflow connection references a missing step".to_string())
}

fn select_source_result(
    sessions: &[AgentSession],
    native_session_id: &str,
    result_id: Option<&str>,
) -> Result<(AgentSession, SessionResult), String> {
    let matching_sessions = sessions
        .iter()
        .filter(|session| session.native_session_id.as_deref() == Some(native_session_id))
        .collect::<Vec<_>>();
    if matching_sessions.is_empty() {
        return Err("The source agent is not connected with a stable native session id".into());
    }
    if let Some(result_id) = result_id.filter(|value| !value.trim().is_empty()) {
        return matching_sessions
            .into_iter()
            .find_map(|session| {
                session
                    .results
                    .iter()
                    .find(|result| result.id == result_id)
                    .map(|result| (session.clone(), result.clone()))
            })
            .ok_or_else(|| "The selected source result is no longer available".to_string());
    }
    if let Some((session, result)) = matching_sessions
        .iter()
        .flat_map(|session| session.results.iter().map(move |result| (*session, result)))
        .max_by_key(|(_, result)| result.created_at)
    {
        return Ok((session.clone(), result.clone()));
    }

    if let Some((session, activity)) = matching_sessions
        .iter()
        .flat_map(|session| {
            session
                .activities
                .iter()
                .filter(|activity| {
                    activity.kind == "message"
                        && activity.status != "running"
                        && activity
                            .detail
                            .as_deref()
                            .is_some_and(|detail| !detail.trim().is_empty())
                })
                .map(move |activity| (*session, activity))
        })
        .max_by_key(|(_, activity)| activity.created_at)
    {
        let response = activity.detail.clone().unwrap_or_default();
        let (mut files, tests) = crate::state::extract_result_artifacts(&response);
        for file in &activity.files {
            if !files.contains(file) {
                files.push(file.clone());
            }
        }
        return Ok((
            session.clone(),
            SessionResult {
                id: format!("archived-message-result:{}", activity.id),
                response,
                created_at: activity.created_at,
                files,
                tests,
            },
        ));
    }

    matching_sessions
        .into_iter()
        .filter_map(|session| {
            session.last_response.as_ref().and_then(|response| {
                (!response.trim().is_empty()).then(|| {
                    let (files, tests) = crate::state::extract_result_artifacts(response);
                    (
                        session.clone(),
                        SessionResult {
                            id: format!("restored-last-response:{}", session.id),
                            response: response.clone(),
                            created_at: session.updated_at,
                            files,
                            tests,
                        },
                    )
                })
            })
        })
        .max_by_key(|(_, result)| result.created_at)
        .ok_or_else(|| "The source agent has no completed result to preview yet".to_string())
}

fn role_label(step: &WorkflowStepDefinition) -> String {
    if step.role == WorkflowRole::Custom && !step.custom_role_label.trim().is_empty() {
        return step.custom_role_label.trim().chars().take(80).collect();
    }
    match step.role {
        WorkflowRole::Planner => "Planner",
        WorkflowRole::Implementer => "Implementer",
        WorkflowRole::Reviewer => "Reviewer",
        WorkflowRole::Tester => "Tester",
        WorkflowRole::Researcher => "Researcher",
        WorkflowRole::Custom => "Custom",
    }
    .into()
}

fn next_instruction(
    target: &WorkflowStepDefinition,
    connection: &WorkflowConnectionDefinition,
    redactions: &mut RedactionCounts,
) -> String {
    let mut sections = Vec::new();
    if !target.instruction.trim().is_empty() {
        sections.push(clean_text(&target.instruction, 4_000, redactions));
    }
    if !target.completion_condition.trim().is_empty() {
        sections.push(format!(
            "Completion condition: {}",
            clean_text(&target.completion_condition, 2_000, redactions)
        ));
    }
    if !connection.additional_instruction.trim().is_empty() {
        sections.push(format!(
            "Connection instruction: {}",
            clean_text(&connection.additional_instruction, 4_000, redactions)
        ));
    }
    sections.join("\n\n")
}

fn collect_files(
    session: &AgentSession,
    result: &SessionResult,
    activities: &[&SessionActivity],
    include_diffs: bool,
    redactions: &mut RedactionCounts,
) -> Vec<WorkflowContextFile> {
    let mut raw_paths = result.files.clone();
    for activity in activities {
        raw_paths.extend(activity.files.iter().cloned());
    }
    let mut seen: HashSet<String> = HashSet::new();
    let mut files = Vec::new();
    for raw_path in raw_paths {
        let (path, external) = match safe_path(&raw_path, session.working_directory.as_deref()) {
            Ok(path) => path,
            Err(PathRejection::Sensitive) => {
                redactions.sensitive_files += 1;
                continue;
            }
            Err(PathRejection::Invalid) => {
                redactions.invalid_file_references += 1;
                continue;
            }
        };
        if seen
            .iter()
            .any(|existing| same_context_path(existing, &path))
        {
            continue;
        }
        seen.insert(path.clone());
        let matching = activities
            .iter()
            .filter(|activity| {
                activity.files.iter().any(|candidate| {
                    safe_path(candidate, session.working_directory.as_deref())
                        .is_ok_and(|(candidate, _)| same_context_path(&candidate, &path))
                })
            })
            .copied()
            .collect::<Vec<_>>();
        let mut seen_details = HashSet::new();
        let details = matching
            .iter()
            .flat_map(|activity| {
                activity_details_for_file(activity, &path, session.working_directory.as_deref())
            })
            .filter(|detail| seen_details.insert(detail.clone()))
            .collect::<Vec<_>>();
        let has_change_activity = matching.iter().any(|activity| {
            activity.kind == "file"
                || activity.title.to_ascii_lowercase().contains("changed file")
                || activity.title.to_ascii_lowercase().contains("edited file")
                || activity
                    .title
                    .to_ascii_lowercase()
                    .contains("arquivo alterado")
        });
        let is_recovered_result = result.id.starts_with("archived-message-result:")
            || result.id.starts_with("restored-last-response:");
        if !is_recovered_result && details.is_empty() && !has_change_activity {
            continue;
        }
        let selected_detail = select_diff_detail(&details);
        let (added, removed) = selected_detail.map(diff_counts).unwrap_or_default();
        let diff = include_diffs
            .then(|| selected_detail.unwrap_or_default().to_string())
            .filter(|value| !value.trim().is_empty())
            .map(|value| clean_text(&value, MAX_DIFF_CHARS, redactions))
            .filter(|value| !value.is_empty());
        files.push(WorkflowContextFile {
            path,
            external,
            added,
            removed,
            diff,
        });
        if files.len() == MAX_FILES {
            redactions.truncated_sections += 1;
            break;
        }
    }
    files
}

fn collect_checks(
    result: &SessionResult,
    activities: &[&SessionActivity],
    redactions: &mut RedactionCounts,
) -> Vec<WorkflowContextCheck> {
    let mut seen = HashSet::new();
    let mut checks = Vec::new();
    for check in &result.tests {
        let summary = clean_text(check, 500, redactions);
        if summary.is_empty() || !seen.insert(normalized_check_key(&summary)) {
            continue;
        }
        checks.push(WorkflowContextCheck { summary });
        if checks.len() == MAX_CHECKS {
            redactions.truncated_sections += 1;
            break;
        }
    }
    for activity in activities {
        let Some(check) = activity_check(activity) else {
            continue;
        };
        let summary = clean_text(&check, 500, redactions);
        if summary.is_empty() || !seen.insert(normalized_check_key(&summary)) {
            continue;
        }
        checks.push(WorkflowContextCheck { summary });
        if checks.len() == MAX_CHECKS {
            redactions.truncated_sections += 1;
            break;
        }
    }
    checks
}

fn collect_activity(
    activities: &[&SessionActivity],
    redactions: &mut RedactionCounts,
) -> Vec<WorkflowContextActivity> {
    let mut seen = HashSet::new();
    let mut selected = activities
        .iter()
        .filter(|activity| {
            !matches!(
                activity.kind.as_str(),
                "prompt" | "message" | "reasoning" | "plan_document" | "file"
            ) && !is_internal_activity(activity)
        })
        .map(|activity| WorkflowContextActivity {
            kind: activity.kind.chars().take(40).collect(),
            title: clean_text(&normalized_activity_title(&activity.title), 180, redactions),
            detail: activity
                .detail
                .as_deref()
                .map(normalized_activity_detail)
                .map(|detail| clean_text(&detail, MAX_ACTIVITY_DETAIL_CHARS, redactions))
                .filter(|detail| !detail.is_empty()),
            status: activity.status.chars().take(40).collect(),
            created_at: activity.created_at,
        })
        .filter(|activity| seen.insert((activity.title.clone(), activity.status.clone())))
        .collect::<Vec<_>>();
    if selected.len() > MAX_ACTIVITIES {
        selected.drain(..selected.len() - MAX_ACTIVITIES);
        redactions.truncated_sections += 1;
    }
    selected
}

fn is_internal_activity(activity: &SessionActivity) -> bool {
    let title = activity.title.trim().to_ascii_lowercase();
    let detail = activity.detail.as_deref().unwrap_or_default().trim();
    title.starts_with("functions ·")
        || title.starts_with("functions:")
        || title == "comando" && (detail.contains("await tools.") || detail.contains("tools."))
        || title.starts_with('{')
        || title.starts_with("***")
        || title.contains("add file:")
        || title == "arquivos alterados"
        || title == "alterações da tarefa"
        || matches!(
            title.as_str(),
            "análise concluída" | "analysis completed" | "plano atualizado" | "plan updated"
        )
        || detail.starts_with("[{\"text\":")
        || detail.contains("\"type\":\"input_text\"")
}

fn normalized_activity_title(title: &str) -> String {
    let title = title.trim();
    for prefix in ["/bin/bash -c '", "bash -c '", "/bin/sh -c '", "sh -c '"] {
        if let Some(command) = title
            .strip_prefix(prefix)
            .and_then(|value| value.strip_suffix('\''))
        {
            return command.trim().to_string();
        }
    }
    for prefix in ["/bin/bash -c \"", "bash -c \"", "/bin/sh -c \"", "sh -c \""] {
        if let Some(command) = title
            .strip_prefix(prefix)
            .and_then(|value| value.strip_suffix('"'))
        {
            return command.trim().to_string();
        }
    }
    title.to_string()
}

fn normalized_activity_detail(detail: &str) -> String {
    let detail = detail.trim();
    if detail.starts_with('"') && detail.ends_with('"') && detail.contains("\\n") {
        return detail[1..detail.len() - 1]
            .replace("\\r\\n", "\n")
            .replace("\\n", "\n")
            .replace("\\t", "\t");
    }
    detail.to_string()
}

fn activity_check(activity: &SessionActivity) -> Option<String> {
    if activity.status != "completed" || is_internal_activity(activity) {
        return None;
    }
    let command = normalized_activity_title(&activity.title);
    let lower = command.to_ascii_lowercase();
    let is_validation = lower.starts_with("wc ")
        || [
            "cargo test",
            "npm test",
            "npm run test",
            "npm run check",
            "npm run build",
            "pnpm test",
            "yarn test",
            "pytest",
            "dotnet test",
            "mvn test",
            "gradle test",
            "flutter test",
            "flutter analyze",
        ]
        .iter()
        .any(|marker| lower.contains(marker));
    if !is_validation {
        return None;
    }
    let raw_output = activity.detail.as_deref().unwrap_or_default().trim();
    let output = raw_output.trim_matches('"').trim_end_matches("\\n").trim();
    Some(if output.is_empty() {
        command
    } else {
        format!("{command}: {output}")
    })
}

fn normalized_check_key(value: &str) -> String {
    value
        .replace("\\n", " ")
        .replace(['"', '\''], "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn same_context_path(left: &str, right: &str) -> bool {
    left == right
        || left
            .strip_suffix(right)
            .is_some_and(|prefix| prefix.ends_with('/'))
        || right
            .strip_suffix(left)
            .is_some_and(|prefix| prefix.ends_with('/'))
}

fn select_diff_detail(details: &[String]) -> Option<&str> {
    details
        .iter()
        .max_by_key(|detail| {
            let rank = if detail.lines().any(|line| line.starts_with("diff --git ")) {
                3
            } else if detail.lines().any(|line| line.starts_with("@@")) {
                2
            } else if detail.contains("\"diff\"") {
                1
            } else {
                0
            };
            (rank, detail.len())
        })
        .map(String::as_str)
}

fn activity_details_for_file(
    activity: &SessionActivity,
    path: &str,
    working_directory: Option<&str>,
) -> Vec<String> {
    let Some(detail) = activity.detail.as_deref() else {
        return Vec::new();
    };
    let sections = patch_sections(detail, working_directory);
    if !sections.is_empty() {
        return sections
            .into_iter()
            .filter_map(|(section_path, detail)| (section_path == path).then_some(detail))
            .collect();
    }
    let matching_paths = activity
        .files
        .iter()
        .filter_map(|candidate| safe_path(candidate, working_directory).ok())
        .filter(|(candidate, _)| same_context_path(candidate, path))
        .count();
    (matching_paths == 1 && activity.files.len() == 1)
        .then(|| detail.to_string())
        .into_iter()
        .collect()
}

fn patch_sections(value: &str, working_directory: Option<&str>) -> Vec<(String, String)> {
    let mut sections = Vec::new();
    let mut current_path: Option<String> = None;
    let mut current_lines = Vec::new();
    let finish_section = |sections: &mut Vec<(String, String)>,
                          current_path: &mut Option<String>,
                          current_lines: &mut Vec<&str>| {
        if let Some(path) = current_path.take() {
            sections.push((path, current_lines.join("\n")));
        }
        current_lines.clear();
    };
    for line in value.lines() {
        let header_path = ["*** Add File: ", "*** Update File: ", "*** Delete File: "]
            .iter()
            .find_map(|prefix| line.strip_prefix(prefix));
        if let Some(raw_path) = header_path {
            finish_section(&mut sections, &mut current_path, &mut current_lines);
            current_path = safe_path(raw_path.trim(), working_directory)
                .ok()
                .map(|(path, _)| path);
            continue;
        }
        if line.starts_with("*** Begin Patch") || line.starts_with("*** End Patch") {
            continue;
        }
        if current_path.is_some() {
            current_lines.push(line);
        }
    }
    finish_section(&mut sections, &mut current_path, &mut current_lines);
    sections
}

fn safe_path(path: &str, working_directory: Option<&str>) -> Result<(String, bool), PathRejection> {
    let path = path.trim().trim_matches(['`', '"', '\'']);
    if path.is_empty()
        || path.len() > 1_024
        || path.contains(['\n', '\r', '\0'])
        || path.contains("***")
        || path.contains("](")
        || path.starts_with("http://")
        || path.starts_with("https://")
    {
        return Err(PathRejection::Invalid);
    }
    if sensitive_path(path) {
        return Err(PathRejection::Sensitive);
    }
    let candidate = Path::new(path);
    if candidate.is_absolute() {
        if let Some(root) = working_directory.map(Path::new) {
            if let Ok(relative) = candidate.strip_prefix(root) {
                let relative = normalize_relative_path(relative)?;
                return Ok((relative, false));
            }
        }
        let name = candidate
            .file_name()
            .ok_or(PathRejection::Invalid)?
            .to_string_lossy();
        return Ok((format!("[external]/{name}"), true));
    }
    Ok((normalize_relative_path(candidate)?, false))
}

fn normalize_relative_path(path: &Path) -> Result<String, PathRejection> {
    use std::path::Component;

    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(value) => parts.push(value.to_string_lossy()),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(PathRejection::Invalid);
            }
        }
    }
    if parts.is_empty() {
        return Err(PathRejection::Invalid);
    }
    Ok(parts.join("/"))
}

fn sensitive_path(path: &str) -> bool {
    Path::new(path).components().any(|component| {
        let value = component.as_os_str().to_string_lossy().to_ascii_lowercase();
        value == ".env"
            || value.starts_with(".env.")
            || matches!(value.as_str(), ".ssh" | ".aws" | ".gnupg")
            || value == "id_rsa"
            || value == "id_ed25519"
            || matches!(
                value.as_str(),
                ".npmrc" | ".pypirc" | ".netrc" | "authorized_keys"
            )
            || value.contains("credentials")
            || value.contains("secrets")
            || value.ends_with(".pem")
            || value.ends_with(".key")
            || value.ends_with(".p12")
            || value.ends_with(".pfx")
    })
}

fn diff_counts(value: &str) -> (usize, usize) {
    value.lines().fold((0, 0), |(added, removed), line| {
        if line.starts_with('+') && !line.starts_with("+++") {
            (added + 1, removed)
        } else if line.starts_with('-') && !line.starts_with("---") {
            (added, removed + 1)
        } else {
            (added, removed)
        }
    })
}

fn clean_text(value: &str, max_chars: usize, redactions: &mut RedactionCounts) -> String {
    let without_ansi = strip_ansi(value);
    let patch_marker_count = without_ansi.matches("*** Begin Patch").count()
        + without_ansi.matches("*** End Patch").count();
    redactions.patch_markers += patch_marker_count;
    let without_patch_markers = without_ansi
        .replace("*** Begin Patch ***", "")
        .replace("*** End Patch ***", "")
        .replace("*** Begin Patch", "")
        .replace("*** End Patch", "");
    let mut lines = Vec::new();
    for line in without_patch_markers.lines() {
        let line = redact_sensitive_file_references(line, redactions);
        lines.push(redact_secret_line(&line, redactions));
    }
    let normalized = lines.join("\n").trim().to_string();
    if normalized.chars().count() <= max_chars {
        return normalized;
    }
    redactions.truncated_sections += 1;
    let mut shortened = normalized.chars().take(max_chars).collect::<String>();
    shortened.push_str("\n… [truncated by Lume]");
    shortened
}

fn redact_sensitive_file_references(line: &str, redactions: &mut RedactionCounts) -> String {
    let mut output = line.to_string();
    let mut search_from = 0;
    while let Some(relative_close) = output[search_from..].find("](") {
        let close = search_from + relative_close;
        let Some(relative_open) = output[..close].rfind('[') else {
            search_from = close + 2;
            continue;
        };
        let target_start = close + 2;
        let Some(relative_end) = output[target_start..].find(')') else {
            break;
        };
        let target_end = target_start + relative_end;
        if sensitive_path(&output[target_start..target_end]) {
            output.replace_range(relative_open..=target_end, "[sensitive file omitted]");
            redactions.sensitive_files += 1;
            search_from = relative_open + "[sensitive file omitted]".len();
        } else {
            search_from = target_end + 1;
        }
    }
    let mentions_sensitive_path = output.split_whitespace().any(|token| {
        let token = token.trim_matches(|character: char| {
            matches!(
                character,
                '`' | '"' | '\'' | '(' | ')' | '[' | ']' | ',' | ';' | ':'
            )
        });
        !token.is_empty() && sensitive_path(token)
    });
    if mentions_sensitive_path {
        redactions.sensitive_files += 1;
        return "[sensitive file reference omitted]".into();
    }
    output
}

fn strip_ansi(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(character) = chars.next() {
        if character != '\u{1b}' {
            output.push(character);
            continue;
        }
        if chars.next_if_eq(&'[').is_some() {
            for code in chars.by_ref() {
                if code.is_ascii_alphabetic() {
                    break;
                }
            }
        }
    }
    output
}

fn redact_secret_line(line: &str, redactions: &mut RedactionCounts) -> String {
    let lower = line.to_ascii_lowercase();
    if let Some(index) = lower.find("bearer ") {
        redactions.secrets += 1;
        return format!("{}Bearer [REDACTED]", &line[..index]);
    }
    for separator in ['=', ':'] {
        let Some((key, value)) = line.split_once(separator) else {
            continue;
        };
        let key = key
            .trim()
            .trim_matches(|character: char| {
                !character.is_ascii_alphanumeric() && character != '_' && character != '-'
            })
            .to_ascii_lowercase()
            .replace(['-', ' '], "_");
        let sensitive_key = [
            "api_key",
            "apikey",
            "access_token",
            "auth_token",
            "password",
            "passwd",
            "client_secret",
            "private_key",
        ]
        .iter()
        .any(|candidate| key.ends_with(candidate))
            || key.ends_with("_token")
            || key.ends_with("_secret")
            || key.ends_with("_password")
            || key.ends_with("_key");
        if sensitive_key && !value.trim().is_empty() {
            redactions.secrets += 1;
            return format!("{}{} [REDACTED]", key.trim(), separator);
        }
    }
    line.to_string()
}

fn redaction_summary(counts: RedactionCounts) -> Vec<WorkflowContextRedaction> {
    let entries = [
        ("secret", "Sensitive values were hidden", counts.secrets),
        (
            "file",
            "Sensitive files were omitted",
            counts.sensitive_files,
        ),
        (
            "invalid_file",
            "Invalid file references were ignored",
            counts.invalid_file_references,
        ),
        (
            "patch",
            "Raw patch markers were removed",
            counts.patch_markers,
        ),
        (
            "limit",
            "Oversized context sections were truncated",
            counts.truncated_sections,
        ),
    ];
    entries
        .into_iter()
        .filter(|(_, _, count)| *count > 0)
        .map(|(kind, summary, count)| WorkflowContextRedaction {
            kind: kind.into(),
            summary: summary.into(),
            count,
        })
        .collect()
}

fn render_markdown(package: &WorkflowContextPackage) -> String {
    let mut sections = vec![format!(
        "# Lume workflow context\n\n- Workflow: `{}`\n- Transition: **{}** → **{}**\n- Policy: `{:?}`\n- Source result: `{}`",
        package.workflow_id,
        package.source_role,
        package.target_role,
        package.policy,
        package.source_result_id
    )];
    sections.push(format!("## Objective\n\n{}", package.objective));
    if let Some(result) = &package.result {
        sections.push(format!("## Source result\n\n{result}"));
    }
    if !package.files.is_empty() {
        let files = package
            .files
            .iter()
            .map(|file| {
                let scope = if file.external { " · external" } else { "" };
                let totals = if file.added > 0 || file.removed > 0 {
                    format!(" (+{} -{})", file.added, file.removed)
                } else {
                    String::new()
                };
                let diff = file
                    .diff
                    .as_deref()
                    .map(|diff| format!("\n\n```diff\n{diff}\n```"))
                    .unwrap_or_default();
                format!("- `{}`{}{}{}", file.path, totals, scope, diff)
            })
            .collect::<Vec<_>>()
            .join("\n");
        sections.push(format!("## Changed files\n\n{files}"));
    }
    if !package.checks.is_empty() {
        sections.push(format!(
            "## Checks\n\n{}",
            package
                .checks
                .iter()
                .map(|check| format!("- {}", check.summary))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    if let Some(plan) = &package.plan {
        sections.push(format!("## Relevant plan\n\n{plan}"));
    }
    if !package.relevant_activity.is_empty() {
        sections.push(format!(
            "## Relevant activity\n\n{}",
            package
                .relevant_activity
                .iter()
                .map(|activity| match activity.detail.as_deref() {
                    Some(detail) => format!(
                        "- **{}** · {}\n  {}",
                        activity.title, activity.status, detail
                    ),
                    None => format!("- **{}** · {}", activity.title, activity.status),
                })
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    if !package.next_instruction.is_empty() {
        sections.push(format!(
            "## Instructions for the next agent\n\n{}",
            package.next_instruction
        ));
    }
    if !package.redactions.is_empty() {
        sections.push(format!(
            "## Context safety\n\n{}",
            package
                .redactions
                .iter()
                .map(|entry| format!("- {}: {}", entry.summary, entry.count))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    sections.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        AccessMode, AgentKind, PermissionProfile, SessionSource, SessionStatus, WorkflowAdvanceMode,
    };

    fn connection(policy: WorkflowContextPolicy) -> WorkflowConnectionDefinition {
        WorkflowConnectionDefinition {
            id: "edge-1".into(),
            from_step_id: "source".into(),
            to_step_id: "target".into(),
            include_response: true,
            include_files: true,
            include_tests: true,
            context_policy: policy,
            context_selection: WorkflowContextSelection::default(),
            additional_instruction: "Review the implementation evidence.".into(),
            requires_approval: true,
            advance_mode: WorkflowAdvanceMode::Manual,
        }
    }

    fn group(policy: WorkflowContextPolicy) -> WorkflowGroupDefinition {
        WorkflowGroupDefinition {
            id: "workflow-1".into(),
            terminal_group_id: "group-1".into(),
            steps: vec![
                WorkflowStepDefinition {
                    id: "source".into(),
                    session_native_id: "thread-source".into(),
                    role: WorkflowRole::Implementer,
                    instruction: "Implement the change.".into(),
                    ..Default::default()
                },
                WorkflowStepDefinition {
                    id: "target".into(),
                    session_native_id: "thread-target".into(),
                    role: WorkflowRole::Reviewer,
                    instruction: "Review the result.".into(),
                    completion_condition: "All findings have evidence.".into(),
                    ..Default::default()
                },
            ],
            connections: vec![connection(policy)],
        }
    }

    fn session() -> AgentSession {
        AgentSession {
            id: "session-1".into(),
            agent: AgentKind::Codex,
            agent_label: "Codex".into(),
            session_name: "Lume".into(),
            project: "Lume".into(),
            source: SessionSource::Cli,
            source_app: None,
            status: SessionStatus::Completed,
            status_label: "Finished".into(),
            started_at: "now".into(),
            updated_at: 300,
            process_id: None,
            native_session_id: Some("thread-source".into()),
            working_directory: Some("/workspace/lume".into()),
            permission_profile: PermissionProfile {
                mode: AccessMode::WorkspaceWrite,
                label: "Workspace".into(),
                approval_policy: "on-request".into(),
                approvals_reviewer: None,
                can_respond_from_lume: true,
                available_actions: Vec::new(),
            },
            pending_permission: None,
            pending_question: None,
            last_response: Some("Implemented".into()),
            results: vec![SessionResult {
                id: "result-1".into(),
                response: "Implemented safely. api_key=super-secret".into(),
                created_at: 300,
                files: vec![
                    "/workspace/lume/src/lib.rs".into(),
                    "/workspace/lume/.env".into(),
                ],
                tests: vec!["cargo test: passed".into()],
            }],
            activities: vec![
                SessionActivity {
                    id: "old".into(),
                    kind: "tool".into(),
                    title: "Old turn".into(),
                    detail: Some("must not leak".into()),
                    status: "completed".into(),
                    created_at: 50,
                    files: Vec::new(),
                    attachments: Vec::new(),
                    append_detail: false,
                },
                SessionActivity {
                    id: "prompt".into(),
                    kind: "prompt".into(),
                    title: "Prompt".into(),
                    detail: Some("Do it".into()),
                    status: "completed".into(),
                    created_at: 100,
                    files: Vec::new(),
                    attachments: Vec::new(),
                    append_detail: false,
                },
                SessionActivity {
                    id: "file".into(),
                    kind: "file".into(),
                    title: "Changed file".into(),
                    detail: Some("*** Begin Patch\n-old\n+new\n*** End Patch".into()),
                    status: "completed".into(),
                    created_at: 200,
                    files: vec!["/workspace/lume/src/lib.rs".into()],
                    attachments: Vec::new(),
                    append_detail: false,
                },
            ],
            rate_limits: Vec::new(),
        }
    }

    #[test]
    fn standard_context_is_turn_scoped_and_redacted() {
        let package = build_context_package(
            &group(WorkflowContextPolicy::Standard),
            "edge-1",
            "Ship the requested change",
            None,
            &[session()],
        )
        .expect("context");
        assert!(package.markdown.contains("Ship the requested change"));
        assert!(package.markdown.contains("src/lib.rs"));
        assert!(!package.markdown.contains("super-secret"));
        assert!(!package.markdown.contains(".env"));
        assert!(!package.markdown.contains("Old turn"));
        assert!(package.plan.is_none());
        assert!(package.relevant_activity.is_empty());
    }

    #[test]
    fn global_context_limit_preserves_instructions_and_truncates_large_results() {
        let mut source = session();
        source.results[0].response = "large result ".repeat(2_000);
        let package = build_context_package_with_limit(
            &group(WorkflowContextPolicy::Detailed),
            "edge-1",
            "Ship the requested change",
            None,
            &[source],
            1_000,
        )
        .expect("limited context");

        assert!(package.estimated_tokens <= 1_000);
        assert!(package
            .markdown
            .contains("## Instructions for the next agent"));
        assert!(package.redactions.iter().any(|entry| entry.kind == "limit"));
    }

    #[test]
    fn standard_context_rejects_patch_paths_and_counts_each_file_section() {
        let patch = concat!(
            "*** Begin Patch\n",
            "*** Update File: /workspace/sample-project/source.txt\n",
            "@@\n",
            " Initial content\n",
            "+Alpha\n",
            "+Beta\n",
            "+Gamma\n",
            "*** Add File: /workspace/sample-project/result.txt\n",
            "+Added Alpha, Beta, and Gamma to source.txt.\n",
            "*** Add File: /workspace/sample-project/.env\n",
            "+LUME_FAKE_TOKEN=example-token-value\n",
            "*** Add File: /workspace/sample-project/test_content.py\n",
            "+from pathlib import Path\n",
            "+\n",
            "+\n",
            "+def test_source_contains_added_lines():\n",
            "+    source_lines = Path(\"source.txt\").read_text().splitlines()\n",
            "+\n",
            "+    assert source_lines[-3:] == [\"Alpha\", \"Beta\", \"Gamma\"]\n",
            "*** End Patch",
        );
        let mut source = session();
        source.working_directory = Some("/workspace/sample-project".into());
        source.results[0].response = concat!(
            "- [source.txt](/workspace/sample-project/source.txt): updated\n",
            "- [.env](/workspace/sample-project/.env): configured\n",
            "api_key=example-secret-value\n",
            "{\"access_token\":\"json-secret\"}",
        )
        .into();
        source.results[0].files = vec![
            patch.into(),
            "source.txt](/workspace/sample-project/source.txt".into(),
        ];
        source.activities[2].detail = Some(patch.into());
        source.activities[2].files = vec![
            "/workspace/sample-project/source.txt".into(),
            "/workspace/sample-project/result.txt".into(),
            "/workspace/sample-project/.env".into(),
            "/workspace/sample-project/test_content.py".into(),
        ];

        let package = build_context_package(
            &group(WorkflowContextPolicy::Standard),
            "edge-1",
            "Review safe context",
            None,
            &[source],
        )
        .expect("context");

        assert!(!package.markdown.contains("example-secret-value"));
        assert!(!package.markdown.contains("json-secret"));
        assert!(!package.markdown.contains("LUME_FAKE_TOKEN"));
        assert!(!package.markdown.contains("*** Begin Patch"));
        assert!(!package
            .markdown
            .contains("](/workspace/sample-project/.env)"));
        assert!(package
            .redactions
            .iter()
            .any(|entry| entry.kind == "invalid_file"));
        assert!(package.redactions.iter().any(|entry| entry.kind == "file"));
        assert_eq!(package.files.len(), 3);
        let file = |path: &str| {
            package
                .files
                .iter()
                .find(|file| file.path == path)
                .expect("reported file")
        };
        assert_eq!(
            (file("source.txt").added, file("source.txt").removed),
            (3, 0)
        );
        assert_eq!(
            (file("result.txt").added, file("result.txt").removed),
            (1, 0)
        );
        assert_eq!(
            (
                file("test_content.py").added,
                file("test_content.py").removed
            ),
            (7, 0)
        );
    }

    #[test]
    fn detailed_context_includes_sanitized_diff_and_activity() {
        let mut source = session();
        let mut duplicate = source.activities[2].clone();
        duplicate.id = "duplicate-file".into();
        duplicate.created_at += 1;
        source.activities.push(duplicate);
        source.activities.push(SessionActivity {
            id: "goal".into(),
            kind: "tool".into(),
            title: "functions · get_goal".into(),
            detail: Some("internal goal state".into()),
            status: "completed".into(),
            created_at: 250,
            files: Vec::new(),
            attachments: Vec::new(),
            append_detail: false,
        });
        source.activities.push(SessionActivity {
            id: "transport".into(),
            kind: "tool".into(),
            title: "Comando".into(),
            detail: Some("const r = await tools.exec_command({cmd: \"wc -l src/lib.rs\"});".into()),
            status: "completed".into(),
            created_at: 260,
            files: Vec::new(),
            attachments: Vec::new(),
            append_detail: false,
        });
        source.activities.push(SessionActivity {
            id: "wrapped-check".into(),
            kind: "command".into(),
            title: "/bin/bash -c 'wc -l src/lib.rs'".into(),
            detail: Some("\"42 src/lib.rs\\n\"".into()),
            status: "completed".into(),
            created_at: 270,
            files: Vec::new(),
            attachments: Vec::new(),
            append_detail: false,
        });
        source.activities.push(SessionActivity {
            id: "plain-check".into(),
            kind: "command".into(),
            title: "wc -l src/lib.rs".into(),
            detail: Some("42 src/lib.rs".into()),
            status: "completed".into(),
            created_at: 271,
            files: Vec::new(),
            attachments: Vec::new(),
            append_detail: false,
        });
        source.activities.push(SessionActivity {
            id: "double-quoted-check".into(),
            kind: "command".into(),
            title: "/bin/bash -c \"wc -l src/lib.rs\"".into(),
            detail: Some("\"42 src/lib.rs\\n\"".into()),
            status: "completed".into(),
            created_at: 272,
            files: Vec::new(),
            attachments: Vec::new(),
            append_detail: false,
        });
        source.activities.push(SessionActivity {
            id: "analysis-summary".into(),
            kind: "tool".into(),
            title: "Análise concluída".into(),
            detail: None,
            status: "completed".into(),
            created_at: 280,
            files: Vec::new(),
            attachments: Vec::new(),
            append_detail: false,
        });
        source.activities.push(SessionActivity {
            id: "plan-summary".into(),
            kind: "plan".into(),
            title: "Plano atualizado".into(),
            detail: Some("✓ Done".into()),
            status: "completed".into(),
            created_at: 281,
            files: Vec::new(),
            attachments: Vec::new(),
            append_detail: false,
        });
        let package = build_context_package(
            &group(WorkflowContextPolicy::Detailed),
            "edge-1",
            "Review the requested change",
            None,
            &[source],
        )
        .expect("context");
        assert!(package.markdown.contains("```diff"));
        assert!(!package.markdown.contains("*** Begin Patch"));
        assert!(!package
            .relevant_activity
            .iter()
            .any(|activity| activity.title == "Changed file"));
        assert_eq!(
            package
                .relevant_activity
                .iter()
                .filter(|activity| activity.title == "wc -l src/lib.rs")
                .count(),
            1
        );
        let wc_checks = package
            .checks
            .iter()
            .filter(|check| check.summary.contains("wc -l src/lib.rs"))
            .collect::<Vec<_>>();
        assert_eq!(wc_checks.len(), 1);
        assert!(!wc_checks[0].summary.contains(['"', '\\']));
        assert!(!package.markdown.contains("await tools.exec_command"));
        assert!(!package.markdown.contains("Análise concluída"));
        assert!(!package.markdown.contains("Plano atualizado"));
        assert!(!package
            .relevant_activity
            .iter()
            .any(|activity| activity.title.contains("get_goal")));
    }

    #[test]
    fn detailed_context_merges_equivalent_paths_and_prefers_unified_diff() {
        let mut source = session();
        source.working_directory = Some("/tmp".into());
        source.results[0].files = vec!["change-summary.txt".into()];
        source
            .activities
            .retain(|activity| activity.kind == "prompt");
        source.activities.push(SessionActivity {
            id: "json-diff".into(),
            kind: "file".into(),
            title: "/workspace/sample-project/change-summary.txt".into(),
            detail: Some(r#"[{"diff":"One\nTwo\nThree\n","kind":{"type":"add"}}]"#.into()),
            status: "completed".into(),
            created_at: 200,
            files: vec!["/workspace/sample-project/change-summary.txt".into()],
            attachments: Vec::new(),
            append_detail: false,
        });
        source.activities.push(SessionActivity {
            id: "git-diff".into(),
            kind: "file".into(),
            title: "Changes".into(),
            detail: Some(concat!(
                "diff --git a/sample-project/change-summary.txt b/sample-project/change-summary.txt\n",
                "--- /dev/null\n",
                "+++ b/sample-project/change-summary.txt\n",
                "@@ -0,0 +1,3 @@\n",
                "+One\n+Two\n+Three"
            ).into()),
            status: "completed".into(),
            created_at: 210,
            files: vec!["sample-project/change-summary.txt".into()],
            attachments: Vec::new(),
            append_detail: false,
        });

        let package = build_context_package(
            &group(WorkflowContextPolicy::Detailed),
            "edge-1",
            "Review",
            None,
            &[source],
        )
        .expect("deduplicated detailed context");

        assert_eq!(package.files.len(), 1);
        assert_eq!((package.files[0].added, package.files[0].removed), (3, 0));
        let diff = package.files[0].diff.as_deref().expect("unified diff");
        assert!(diff.contains("diff --git"));
        assert!(!diff.contains("\"kind\""));
    }

    #[test]
    fn live_context_does_not_report_a_read_only_file_as_changed_without_diffs() {
        let mut source = session();
        source.results[0].files = vec!["detailed-live-test.txt".into()];
        source
            .activities
            .retain(|activity| activity.kind == "prompt");
        source.activities.push(SessionActivity {
            id: "read-only".into(),
            kind: "command".into(),
            title: "sed -n '1,10p' detailed-live-test.txt".into(),
            detail: Some("Red\nGreen\nBlue".into()),
            status: "completed".into(),
            created_at: 200,
            files: Vec::new(),
            attachments: Vec::new(),
            append_detail: false,
        });

        let package = build_context_package(
            &group(WorkflowContextPolicy::Standard),
            "edge-1",
            "Review",
            None,
            &[source.clone()],
        )
        .expect("read-only standard context");

        assert!(package.files.is_empty());

        let mut custom = group(WorkflowContextPolicy::Custom);
        custom.connections[0].context_selection = WorkflowContextSelection {
            response: true,
            files: true,
            checks: false,
            plan: false,
            activity: false,
            diffs: false,
        };
        let package = build_context_package(&custom, "edge-1", "Review", None, &[source])
            .expect("read-only custom context");
        assert!(package.files.is_empty());
    }

    #[test]
    fn custom_policy_respects_individual_selection() {
        let mut group = group(WorkflowContextPolicy::Custom);
        group.connections[0].context_selection = WorkflowContextSelection {
            response: false,
            files: false,
            checks: true,
            plan: false,
            activity: false,
            diffs: false,
        };
        let package = build_context_package(&group, "edge-1", "Run checks", None, &[session()])
            .expect("context");
        assert!(package.result.is_none());
        assert!(package.files.is_empty());
        assert_eq!(package.checks.len(), 1);
    }

    #[test]
    fn source_must_match_the_native_session_id() {
        let mut session = session();
        session.native_session_id = None;
        let error = build_context_package(
            &group(WorkflowContextPolicy::Standard),
            "edge-1",
            "Review",
            None,
            &[session],
        )
        .expect_err("must reject provisional sessions");
        assert!(error.contains("stable native session id"));
    }

    #[test]
    fn source_result_is_found_when_an_earlier_duplicate_session_is_empty() {
        let mut empty_duplicate = session();
        empty_duplicate.id = "session-empty".into();
        empty_duplicate.results.clear();
        empty_duplicate.activities.clear();
        let mut completed = session();
        completed.results[0].response = "Completed duplicate result.".into();

        let package = build_context_package(
            &group(WorkflowContextPolicy::Standard),
            "edge-1",
            "Review",
            None,
            &[empty_duplicate, completed],
        )
        .expect("context from completed duplicate");

        assert_eq!(package.source_result_id, "result-1");
        assert!(package.markdown.contains("Completed duplicate result."));
    }

    #[test]
    fn explicitly_selected_result_is_found_across_duplicate_sessions() {
        let mut empty_duplicate = session();
        empty_duplicate.id = "session-empty".into();
        empty_duplicate.results.clear();
        let completed = session();

        let package = build_context_package(
            &group(WorkflowContextPolicy::Standard),
            "edge-1",
            "Review",
            Some("result-1"),
            &[empty_duplicate, completed],
        )
        .expect("selected result from completed duplicate");

        assert_eq!(package.source_result_id, "result-1");
    }

    #[test]
    fn archived_agent_message_is_used_when_session_results_were_not_restored() {
        let mut archived = session();
        archived.results.clear();
        archived.last_response = None;
        archived
            .activities
            .retain(|activity| activity.kind != "message");
        archived.activities.push(SessionActivity {
            id: "archived-final".into(),
            kind: "message".into(),
            title: "Agent response".into(),
            detail: Some("Final response restored from chat history.".into()),
            status: "completed".into(),
            created_at: 350,
            files: vec!["/workspace/lume/src/lib.rs".into()],
            attachments: Vec::new(),
            append_detail: false,
        });

        let package = build_context_package(
            &group(WorkflowContextPolicy::Standard),
            "edge-1",
            "Review",
            None,
            &[archived],
        )
        .expect("context from archived final message");

        assert_eq!(
            package.source_result_id,
            "archived-message-result:archived-final"
        );
        assert!(package
            .markdown
            .contains("Final response restored from chat history."));
        assert!(package.markdown.contains("src/lib.rs"));
    }

    #[test]
    fn claude_code_results_use_the_same_safe_package_contract() {
        let mut session = session();
        session.agent = AgentKind::ClaudeCode;
        session.agent_label = "Claude Code".into();
        session.activities[2].kind = "tool".into();
        let package = build_context_package(
            &group(WorkflowContextPolicy::Standard),
            "edge-1",
            "Review Claude's result",
            Some("result-1"),
            &[session],
        )
        .expect("context");

        assert_eq!(package.source_result_id, "result-1");
        assert!(package.markdown.contains("cargo test: passed"));
        assert!(!package.markdown.contains("super-secret"));
    }
}
