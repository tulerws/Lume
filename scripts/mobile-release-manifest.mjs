// Gera o `mobile-latest.json` que o atualizador do Android lê.
//
// O formato **não é negociável**: aparelhos com as versões 0.5.0 a 0.5.3 já
// instaladas leem este arquivo para descobrir que existe versão nova. Renomear um
// campo aqui deixa a base instalada sem caminho de atualização, e essas pessoas
// não têm como receber uma correção — porque receber correções é justamente o que
// teria quebrado.
//
// O consumidor está em
// `android/app/src/main/java/com/tulerws/lume/mobile/data/update/ManifestoDeAtualizacao.kt`,
// e `AtualizadorTest` fixa o formato contra uma cópia do arquivo publicado.
//
//   node scripts/mobile-release-manifest.mjs \
//     --version 0.5.4 --apk caminho/Lume-Mobile.apk \
//     --repository tulerws/Lume --tag v0.5.4 --output mobile-latest.json

import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import { basename } from "node:path";

const obrigatorios = ["--version", "--apk", "--repository", "--tag", "--output"];

const valores = new Map();
for (let i = 2; i < process.argv.length; i += 2) {
  valores.set(process.argv[i], process.argv[i + 1]);
}

const faltando = obrigatorios.filter((chave) => !valores.get(chave));
if (faltando.length > 0) {
  console.error(`mobile-release-manifest: faltam argumentos: ${faltando.join(", ")}`);
  process.exit(1);
}

const version = valores.get("--version");
const apkPath = valores.get("--apk");
const repository = valores.get("--repository");
const tag = valores.get("--tag");
const output = valores.get("--output");

const apk = await readFile(apkPath);
const sha256 = createHash("sha256").update(apk).digest("hex");

// O nome do arquivo entra na URL como está. É por isso que o passo que copia o
// APK o chama de `Lume-Mobile.apk`: esse nome é parte do endereço que o
// atualizador vai buscar.
const apkName = basename(apkPath);

const manifest = {
  version,
  publishedAt: new Date().toISOString(),
  notesUrl: `https://github.com/${repository}/releases/tag/${tag}`,
  android: {
    url: `https://github.com/${repository}/releases/download/${tag}/${apkName}`,
    sha256,
  },
};

await writeFile(output, `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`mobile-latest.json: ${version} (${sha256.slice(0, 12)}…)`);
