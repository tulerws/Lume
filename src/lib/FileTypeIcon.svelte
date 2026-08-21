<script lang="ts">
  import Icon from "@iconify/svelte";
  import type { IconifyIcon } from "@iconify/types";
  import dartIcon from "@iconify-icons/logos/dart";
  import cIcon from "@iconify-icons/logos/c";
  import cppIcon from "@iconify-icons/logos/c-plusplus";
  import kotlinIcon from "@iconify-icons/logos/kotlin-icon";
  import svelteIcon from "@iconify-icons/logos/svelte-icon";
  import htmlIcon from "@iconify-icons/logos/html-5";
  import cssIcon from "@iconify-icons/logos/css-3-official";
  import javascriptIcon from "@iconify-icons/logos/javascript";
  import typescriptIcon from "@iconify-icons/logos/typescript-icon";
  import rustIcon from "@iconify-icons/logos/rust";
  import pythonIcon from "@iconify-icons/logos/python";
  import goIcon from "@iconify-icons/logos/go";
  import phpIcon from "@iconify-icons/logos/php";
  import rubyIcon from "@iconify-icons/logos/ruby";
  import swiftIcon from "@iconify-icons/logos/swift";
  import dockerIcon from "@iconify-icons/logos/docker-icon";
  import jsonIcon from "@iconify-icons/logos/json";
  import yamlIcon from "@iconify-icons/logos/yaml";
  import markdownIcon from "@iconify-icons/logos/markdown";
  import vueIcon from "@iconify-icons/logos/vue";
  import astroIcon from "@iconify-icons/logos/astro-icon";
  import bashIcon from "@iconify-icons/logos/bash-icon";
  import tomlIcon from "@iconify-icons/logos/toml";
  import scalaIcon from "@iconify-icons/logos/scala";
  import luaIcon from "@iconify-icons/logos/lua";
  import perlIcon from "@iconify-icons/logos/perl";
  import erlangIcon from "@iconify-icons/logos/erlang";
  import haskellIcon from "@iconify-icons/logos/haskell-icon";
  import clojureIcon from "@iconify-icons/logos/clojure";
  import zigIcon from "@iconify-icons/logos/zig";
  import solidityIcon from "@iconify-icons/logos/solidity";
  import juliaIcon from "@iconify-icons/logos/julia";
  import webassemblyIcon from "@iconify-icons/logos/webassembly";
  import graphqlIcon from "@iconify-icons/logos/graphql";
  import prismaIcon from "@iconify-icons/logos/prisma";
  import gradleIcon from "@iconify-icons/logos/gradle";
  import rIcon from "@iconify-icons/logos/r-lang";
  import fsharpIcon from "@iconify-icons/logos/fsharp";
  import ocamlIcon from "@iconify-icons/logos/ocaml";
  import crystalIcon from "@iconify-icons/logos/crystal";
  import fortranIcon from "@iconify-icons/logos/fortran";
  import terraformIcon from "@iconify-icons/logos/terraform";
  import npmIcon from "@iconify-icons/logos/npm-icon";
  import yarnIcon from "@iconify-icons/logos/yarn";
  import pnpmIcon from "@iconify-icons/logos/pnpm";
  import bunIcon from "@iconify-icons/logos/bun";
  import denoIcon from "@iconify-icons/logos/deno";
  import viteIcon from "@iconify-icons/logos/vitejs";
  import webpackIcon from "@iconify-icons/logos/webpack";
  import eslintIcon from "@iconify-icons/logos/eslint";
  import prettierIcon from "@iconify-icons/logos/prettier";
  import tailwindIcon from "@iconify-icons/logos/tailwindcss";

  let { path, size = 14 } = $props<{ path: string; size?: number }>();

  // Exact paths from Icons - Maintained by Yusif Aliyev (MIT).
  const CSHARP_PATH = "m11.5 15.97.41 2.44c-.26.14-.68.27-1.24.39-.57.13-1.24.2-2.01.2-2.21-.04-3.87-.7-4.98-1.96C2.56 15.77 2 14.16 2 12.21c.05-2.31.72-4.08 2-5.32C5.32 5.64 6.96 5 8.94 5c.75 0 1.4.07 1.94.19s.94.25 1.2.4l-.58 2.49-1.06-.34c-.4-.1-.86-.15-1.39-.15-1.16-.01-2.12.36-2.87 1.1-.76.73-1.15 1.85-1.18 3.34 0 1.36.37 2.42 1.08 3.2.71.77 1.71 1.17 2.99 1.18l1.33-.12c.43-.08.79-.19 1.1-.32M13.89 19l.61-4H13l.34-2h1.5l.32-2h-1.5L14 9h1.5l.61-4h2l-.61 4h1l.61-4h2l-.61 4H22l-.34 2h-1.5l-.32 2h1.5L21 15h-1.5l-.61 4h-2l.61-4h-1l-.61 4h-2m2.95-6h1l.32-2h-1l-.32 2z";
  const JAVA_PATH = "M2 21h18v-2H2M20 8h-2V5h2m0-2H4v10a4 4 0 0 0 4 4h6a4 4 0 0 0 4-4v-3h2a2 2 0 0 0 2-2V5a2 2 0 0 0-2-2z";

  const extensionIcons: Record<string, IconifyIcon> = {
    dart: dartIcon,
    c: cIcon,
    h: cIcon,
    cc: cppIcon,
    cpp: cppIcon,
    cxx: cppIcon,
    hpp: cppIcon,
    hxx: cppIcon,
    kt: kotlinIcon,
    kts: kotlinIcon,
    svelte: svelteIcon,
    html: htmlIcon,
    htm: htmlIcon,
    css: cssIcon,
    scss: cssIcon,
    sass: cssIcon,
    less: cssIcon,
    js: javascriptIcon,
    jsx: javascriptIcon,
    mjs: javascriptIcon,
    cjs: javascriptIcon,
    ts: typescriptIcon,
    tsx: typescriptIcon,
    mts: typescriptIcon,
    cts: typescriptIcon,
    rs: rustIcon,
    py: pythonIcon,
    pyw: pythonIcon,
    pyi: pythonIcon,
    go: goIcon,
    php: phpIcon,
    rb: rubyIcon,
    swift: swiftIcon,
    json: jsonIcon,
    jsonc: jsonIcon,
    yaml: yamlIcon,
    yml: yamlIcon,
    md: markdownIcon,
    mdx: markdownIcon,
    vue: vueIcon,
    astro: astroIcon,
    sh: bashIcon,
    bash: bashIcon,
    toml: tomlIcon,
    scala: scalaIcon,
    sc: scalaIcon,
    lua: luaIcon,
    pl: perlIcon,
    pm: perlIcon,
    erl: erlangIcon,
    hrl: erlangIcon,
    hs: haskellIcon,
    lhs: haskellIcon,
    clj: clojureIcon,
    cljs: clojureIcon,
    cljc: clojureIcon,
    edn: clojureIcon,
    zig: zigIcon,
    sol: solidityIcon,
    jl: juliaIcon,
    wasm: webassemblyIcon,
    wat: webassemblyIcon,
    graphql: graphqlIcon,
    gql: graphqlIcon,
    prisma: prismaIcon,
    gradle: gradleIcon,
    r: rIcon,
    rmd: rIcon,
    fs: fsharpIcon,
    fsx: fsharpIcon,
    fsi: fsharpIcon,
    ml: ocamlIcon,
    mli: ocamlIcon,
    cr: crystalIcon,
    f: fortranIcon,
    f90: fortranIcon,
    f95: fortranIcon,
    tf: terraformIcon,
    tfvars: terraformIcon,
    "vite-config": viteIcon,
    "webpack-config": webpackIcon,
    "eslint-config": eslintIcon,
    "prettier-config": prettierIcon,
    "tailwind-config": tailwindIcon,
  };

  const filenameIcons: Record<string, IconifyIcon> = {
    "package.json": npmIcon,
    "package-lock.json": npmIcon,
    "npm-shrinkwrap.json": npmIcon,
    "yarn.lock": yarnIcon,
    "pnpm-lock.yaml": pnpmIcon,
    "bun.lock": bunIcon,
    "bun.lockb": bunIcon,
    "deno.json": denoIcon,
    "deno.jsonc": denoIcon,
    "cargo.toml": rustIcon,
    "cargo.lock": rustIcon,
    "pubspec.yaml": dartIcon,
    "pubspec.lock": dartIcon,
    "build.gradle": gradleIcon,
    "settings.gradle": gradleIcon,
    "chart.yaml": yamlIcon,
  };

  const type = $derived(fileType(path));
  const icon = $derived(type === "docker" ? dockerIcon : filenameIcons[type] ?? extensionIcons[type] ?? null);
  const maintainedPath = $derived(type === "cs" ? CSHARP_PATH : type === "java" ? JAVA_PATH : null);
  const displaySize = $derived(type === "go" || type === "php" ? size + 2 : size);

  function fileType(value: string): string {
    const name = value.replaceAll("\\", "/").split("/").pop()?.toLowerCase() ?? "";
    if (name === "dockerfile" || name === "containerfile") return "docker";
    if (name === "pom.xml") return "java";
    if (filenameIcons[name]) return name;
    if (name.startsWith("vite.config.")) return "vite-config";
    if (name.startsWith("webpack.config.")) return "webpack-config";
    if (name.startsWith("eslint.config.") || name.startsWith(".eslintrc")) return "eslint-config";
    if (name.startsWith("prettier.config.") || name.startsWith(".prettierrc")) return "prettier-config";
    if (name.startsWith("tailwind.config.")) return "tailwind-config";
    const extension = name.includes(".") ? name.split(".").pop() ?? "" : "";
    if (extension === "cs" || extension === "csx") return "cs";
    if (["java", "class", "jar"].includes(extension)) return "java";
    return extension;
  }
</script>

<span class:brand={Boolean(icon)} class:maintained={Boolean(maintainedPath)} class="file-type-icon" aria-hidden="true" style:width={`${size}px`} style:height={`${size}px`}>
  {#if maintainedPath}
    <svg class="maintained-icon" viewBox="0 0 24 24" style:width={`${displaySize}px`} style:height={`${displaySize}px`}><path d={maintainedPath} /></svg>
  {:else if icon}
    <Icon {icon} width={displaySize} height={displaySize} />
  {:else}
    <svg viewBox="0 0 20 20">
      <path d="M4 3h8l4 4v10H4Z" /><path d="M12 3v4h4" />
    </svg>
  {/if}
</span>

<style>
  .file-type-icon { display: inline-grid; place-items: center; flex: 0 0 auto; color: inherit; opacity: .72; }
  .file-type-icon.brand { filter: grayscale(1) contrast(.8); opacity: .78; }
  .file-type-icon.brand :global(svg),
  .file-type-icon.brand :global(svg *) { stroke: none !important; stroke-width: 0 !important; }
  .file-type-icon.maintained { color: inherit; opacity: .76; }
  svg { width: 100%; height: 100%; fill: none; stroke: currentColor; stroke-width: 1.35; stroke-linecap: round; stroke-linejoin: round; }
  .maintained-icon { fill: currentColor; stroke: none; }
  :global(.terminal-window.dark) .file-type-icon.brand { filter: grayscale(1) invert(.78) contrast(.82); opacity: .8; }
</style>
