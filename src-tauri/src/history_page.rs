//! Paginação do histórico para o controle remoto.
//!
//! Puro: recebe a janela que o `Store` devolveu e recorta uma página dela. Não
//! conhece soquete nem banco, e por isso o cálculo do cursor — que é a parte
//! sutil — é testável sem subir servidor.
//!
//! **Quem ordena é este módulo, não a consulta.** O `store.rs` faz
//! `ORDER BY created_at DESC` **sem desempate**, e entre registros do mesmo
//! milissegundo a ordem do SQLite é indefinida e pode variar entre execuções.
//! Um cursor pousado exatamente sobre um empate pularia ou repetiria entrada
//! entre páginas. Reordenar aqui custa uma comparação por registro numa janela
//! de no máximo 200.

use serde::{Deserialize, Serialize};

use crate::domain::HistoryEntry;

/// A janela que o servidor lê do banco.
///
/// É o mesmo teto que `AppState::history` já aplica (`limit.min(200)`), e não
/// um limite novo: o celular alcança exatamente o que o desktop alcança.
pub const CEILING: usize = 200;

/// Quantas entradas devolver quando o aplicativo não pede um número.
pub const DEFAULT_LIMIT: usize = 50;

/// Teto por página. Passar disso não traria mais dados — a janela inteira tem
/// 200 — e só aumentaria a mensagem.
pub const MAX_LIMIT: usize = 100;

/// Onde a página anterior parou.
///
/// Leva o instante **e** o identificador porque o instante sozinho não
/// desempata: este projeto já teve dois registros no mesmo milissegundo, e com
/// cursor só de horário eles caem os dois na mesma página ou os dois fora.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cursor {
    pub created_at: i64,
    pub id: String,
}

impl Cursor {
    fn of(entry: &HistoryEntry) -> Self {
        Self {
            created_at: entry.created_at,
            id: entry.id.clone(),
        }
    }

    /// Verdadeiro para o que vem **depois** deste cursor na ordem de exibição.
    fn precedes(&self, entry: &HistoryEntry) -> bool {
        (entry.created_at, entry.id.as_str()) < (self.created_at, self.id.as_str())
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub entries: Vec<HistoryEntry>,
    /// `None` quando não há mais o que devolver. Serializa como `null`, e não
    /// some do JSON: o aplicativo distingue "acabou" de "campo esquecido".
    pub next_cursor: Option<Cursor>,
    /// `true` quando `next_cursor` é nulo **por causa do teto**, e não por fim
    /// real dos dados. Existe para o aplicativo poder dizer "estes são os 200
    /// mais recentes" em vez de sugerir que o histórico terminou ali.
    pub at_ceiling: bool,
}

/// Recorta uma página da janela.
///
/// `window` vem do `AppState::history(CEILING)`, em qualquer ordem.
pub fn page(mut window: Vec<HistoryEntry>, cursor: Option<&Cursor>, limit: usize) -> Page {
    // Janela cheia significa que o banco tinha pelo menos o teto de registros.
    // Aqui mora um falso positivo assumido: com exatamente 200 registros no
    // banco, isto diz "há mais" quando não há. Erra para o lado seguro — nunca
    // afirma que o histórico acabou quando ele pode continuar.
    let saturated = window.len() >= CEILING;

    // Mais recente primeiro, desempatado pelo identificador decrescente.
    window.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| right.id.cmp(&left.id))
    });

    if let Some(cursor) = cursor {
        window.retain(|entry| cursor.precedes(entry));
    }

    // Uma página de zero entradas não informa nada e ainda faria o aplicativo
    // concluir que o histórico está vazio.
    let limit = limit.clamp(1, MAX_LIMIT);
    let remaining = window.len() > limit;
    window.truncate(limit);

    let next_cursor = remaining.then(|| window.last().map(Cursor::of)).flatten();
    Page {
        at_ceiling: next_cursor.is_none() && saturated,
        next_cursor,
        entries: window,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, created_at: i64) -> HistoryEntry {
        HistoryEntry {
            id: id.to_string(),
            session_id: "s-1".to_string(),
            agent_label: "Claude".to_string(),
            project: "Lume".to_string(),
            event: "completed".to_string(),
            summary: "Tarefa finalizada".to_string(),
            created_at,
        }
    }

    /// Instantes decrescentes, do mais recente para o mais antigo.
    fn window(count: usize) -> Vec<HistoryEntry> {
        (0..count)
            .map(|index| entry(&format!("h-{index:03}"), 1_000_000 - index as i64))
            .collect()
    }

    fn ids(page: &Page) -> Vec<&str> {
        page.entries.iter().map(|entry| entry.id.as_str()).collect()
    }

    #[test]
    fn the_first_page_is_the_most_recent() {
        let result = page(window(10), None, 3);

        assert_eq!(ids(&result), vec!["h-000", "h-001", "h-002"]);
        assert_eq!(result.next_cursor, Some(Cursor::of(&entry("h-002", 999_998))));
        assert!(!result.at_ceiling);
    }

    #[test]
    fn the_incoming_order_does_not_matter() {
        let mut shuffled = window(10);
        shuffled.reverse();
        shuffled.swap(0, 5);

        // O `store.rs` ordena sem desempate, então a ordem que chega aqui não é
        // garantida. A página não pode depender dela.
        assert_eq!(ids(&page(shuffled, None, 3)), vec!["h-000", "h-001", "h-002"]);
    }

    #[test]
    fn entries_in_the_same_instant_break_the_tie_by_id() {
        let tied = vec![
            entry("h-a", 500),
            entry("h-c", 500),
            entry("h-b", 500),
            entry("h-z", 400),
        ];

        // Identificador decrescente dentro do mesmo milissegundo.
        assert_eq!(ids(&page(tied, None, 4)), vec!["h-c", "h-b", "h-a", "h-z"]);
    }

    #[test]
    fn the_cursor_resumes_across_a_tie_without_gap_or_repeat() {
        // O caso que o desempate existe para resolver: a fronteira da página cai
        // no meio de três registros do mesmo instante.
        let tied = || {
            vec![
                entry("h-a", 500),
                entry("h-b", 500),
                entry("h-c", 500),
                entry("h-d", 400),
            ]
        };

        let first = page(tied(), None, 2);
        assert_eq!(ids(&first), vec!["h-c", "h-b"]);

        let second = page(tied(), first.next_cursor.as_ref(), 2);
        assert_eq!(ids(&second), vec!["h-a", "h-d"]);

        // Sem buraco e sem repetição: as duas páginas cobrem tudo, uma vez cada.
        assert_eq!(second.next_cursor, None);
    }

    #[test]
    fn walking_the_cursor_covers_the_window_exactly_once() {
        let mut seen = Vec::new();
        let mut cursor = None;
        loop {
            let result = page(window(10), cursor.as_ref(), 3);
            seen.extend(ids(&result).into_iter().map(str::to_string));
            match result.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }

        assert_eq!(seen.len(), 10);
        assert_eq!(seen, ids(&page(window(10), None, MAX_LIMIT)));
    }

    #[test]
    fn the_last_page_reports_no_cursor() {
        let result = page(window(3), None, 10);

        assert_eq!(result.entries.len(), 3);
        assert_eq!(result.next_cursor, None);
        // Janela curta: o fim é o fim de verdade, não o teto do servidor.
        assert!(!result.at_ceiling);
    }

    #[test]
    fn a_full_window_that_runs_out_reports_the_ceiling() {
        let result = page(window(CEILING), None, MAX_LIMIT);
        assert!(!result.at_ceiling, "ainda há página seguinte");

        let last = page(window(CEILING), result.next_cursor.as_ref(), MAX_LIMIT);
        assert_eq!(last.next_cursor, None);
        // Sem isto o aplicativo diria "fim do histórico" onde há só o teto.
        assert!(last.at_ceiling);
    }

    #[test]
    fn the_limit_is_capped_by_the_server() {
        let result = page(window(CEILING), None, 5_000);

        assert_eq!(result.entries.len(), MAX_LIMIT);
    }

    #[test]
    fn a_limit_of_zero_still_returns_something() {
        // Página vazia com cursor nulo seria lida como "o histórico acabou".
        let result = page(window(10), None, 0);

        assert_eq!(result.entries.len(), 1);
        assert!(result.next_cursor.is_some());
    }

    #[test]
    fn a_cursor_past_the_end_returns_nothing() {
        let result = page(
            window(10),
            Some(&Cursor {
                created_at: 0,
                id: String::new(),
            }),
            5,
        );

        assert!(result.entries.is_empty());
        assert_eq!(result.next_cursor, None);
    }

    #[test]
    fn a_cursor_that_left_the_window_still_lands_somewhere_sane() {
        // Cursor guardado pelo aplicativo, de um registro que já saiu dos 200
        // mais recentes. Não existe na janela, mas a comparação continua válida.
        let result = page(window(10), Some(&Cursor { created_at: 999_995, id: "h-005".into() }), 3);

        assert_eq!(ids(&result), vec!["h-006", "h-007", "h-008"]);
    }
}
