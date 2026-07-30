import type { PromptAttachmentInput } from "$lib/domain";

const MAX_PASTED_IMAGE_BYTES = 20 * 1024 * 1024;
const MAX_PASTED_FILE_BYTES = 25 * 1024 * 1024;
const MAX_ATTACHMENT_DATA_URL_LENGTH = 6_900_000;

function message(language: "en" | "pt-BR", english: string, portuguese: string) {
  return language === "pt-BR" ? portuguese : english;
}

function imageExtension(mimeType: string) {
  return {
    "image/gif": "gif",
    "image/jpeg": "jpg",
    "image/png": "png",
    "image/webp": "webp",
  }[mimeType] ?? "img";
}

function loadImage(
  source: string,
  language: "en" | "pt-BR",
): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.onload = () => resolve(image);
    image.onerror = () => reject(new Error(message(
      language,
      "Could not preview this image",
      "Não foi possível visualizar esta imagem",
    )));
    image.src = source;
  });
}

function readFileDataUrl(
  file: File,
  language: "en" | "pt-BR",
): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () =>
      typeof reader.result === "string"
        ? resolve(reader.result)
        : reject(new Error(message(
            language,
            "Could not read this file",
            "Não foi possível ler este arquivo",
          )));
    reader.onerror = () => reject(new Error(message(
      language,
      "Could not read this file",
      "Não foi possível ler este arquivo",
    )));
    reader.readAsDataURL(file);
  });
}

function resizedImageDataUrl(
  image: HTMLImageElement,
  maxDimension: number,
  quality: number,
  language: "en" | "pt-BR",
) {
  const scale = Math.min(1, maxDimension / Math.max(image.naturalWidth, image.naturalHeight));
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(image.naturalWidth * scale));
  canvas.height = Math.max(1, Math.round(image.naturalHeight * scale));
  const context = canvas.getContext("2d");
  if (!context) {
    throw new Error(message(
      language,
      "Could not prepare this image",
      "Não foi possível preparar esta imagem",
    ));
  }
  context.fillStyle = "#ffffff";
  context.fillRect(0, 0, canvas.width, canvas.height);
  context.drawImage(image, 0, 0, canvas.width, canvas.height);
  return canvas.toDataURL("image/jpeg", quality);
}

export async function createImagePreview(
  source: string,
  language: "en" | "pt-BR",
) {
  return resizedImageDataUrl(await loadImage(source, language), 480, 0.78, language);
}

export async function prepareClipboardImage(
  file: File,
  index: number,
  language: "en" | "pt-BR",
): Promise<PromptAttachmentInput> {
  if (file.size > MAX_PASTED_IMAGE_BYTES) {
    throw new Error(message(
      language,
      "The pasted image is too large",
      "A imagem colada é muito grande",
    ));
  }
  const image = await loadImage(await readFileDataUrl(file, language), language);
  let dataUrl = "";
  for (const [dimension, quality] of [[1600, 0.82], [1400, 0.74], [1200, 0.68], [960, 0.6]]) {
    dataUrl = resizedImageDataUrl(image, dimension, quality, language);
    if (dataUrl.length <= MAX_ATTACHMENT_DATA_URL_LENGTH) break;
  }
  if (dataUrl.length > MAX_ATTACHMENT_DATA_URL_LENGTH) {
    throw new Error(message(
      language,
      "The pasted image is too large",
      "A imagem colada é muito grande",
    ));
  }
  const baseName = (file.name || `clipboard-image-${index + 1}`)
    .replace(/\.[^.]+$/, "");
  return {
    name: `${baseName}.jpg`,
    mimeType: "image/jpeg",
    dataBase64: dataUrl.slice(dataUrl.indexOf(",") + 1),
    previewDataUrl: resizedImageDataUrl(image, 480, 0.78, language),
  };
}

function directImageFiles(event: ClipboardEvent) {
  const itemFiles = [...(event.clipboardData?.items ?? [])]
    .filter((item) => item.kind === "file" && item.type.startsWith("image/"))
    .map((item) => item.getAsFile())
    .filter((file): file is File => file !== null);
  if (itemFiles.length) return itemFiles;
  return [...(event.clipboardData?.files ?? [])]
    .filter((file) => !file.type || file.type.startsWith("image/"));
}

function directClipboardFiles(event: ClipboardEvent) {
  const itemFiles = [...(event.clipboardData?.items ?? [])]
    .filter((item) => item.kind === "file")
    .map((item) => item.getAsFile())
    .filter((file): file is File => file !== null);
  return itemFiles.length
    ? itemFiles
    : [...(event.clipboardData?.files ?? [])];
}

function dataUrlImageFiles(event: ClipboardEvent) {
  const html = event.clipboardData?.getData("text/html") ?? "";
  const matches = [...html.matchAll(/<img[^>]+src=["'](data:image\/[^"']+)["']/gi)];
  return matches.flatMap((match, index) => {
    const dataUrl = match[1].replaceAll("&amp;", "&");
    const parsed = /^data:(image\/[^;,]+);base64,(.+)$/i.exec(dataUrl);
    if (!parsed) return [];
    try {
      const binary = atob(parsed[2]);
      const bytes = new Uint8Array(binary.length);
      for (let offset = 0; offset < binary.length; offset += 1) {
        bytes[offset] = binary.charCodeAt(offset);
      }
      return [new File(
        [bytes],
        `clipboard-image-${index + 1}.${imageExtension(parsed[1])}`,
        { type: parsed[1] },
      )];
    } catch {
      return [];
    }
  });
}

function fileUriPath(uri: string) {
  try {
    const parsed = new URL(uri);
    if (parsed.protocol !== "file:") return null;
    let path = decodeURIComponent(parsed.pathname);
    if (/^\/[a-zA-Z]:\//.test(path)) path = path.slice(1);
    return path;
  } catch {
    return null;
  }
}

function clipboardFilePaths(event: ClipboardEvent) {
  const uriList = event.clipboardData?.getData("text/uri-list") ||
    event.clipboardData?.getData("text/plain") ||
    "";
  return uriList
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line && !line.startsWith("#"))
    .map(fileUriPath)
    .filter((path): path is string => Boolean(path));
}

function clipboardImagePaths(event: ClipboardEvent) {
  return clipboardFilePaths(event)
    .filter((path) => isImageAttachmentPath(path));
}

async function asynchronousClipboardImages() {
  if (!navigator.clipboard?.read) return [];
  const files: File[] = [];
  const clipboardItems = await navigator.clipboard.read();
  for (const item of clipboardItems) {
    const imageType = item.types.find((type) => type.startsWith("image/"));
    if (!imageType) continue;
    const blob = await item.getType(imageType);
    files.push(new File(
      [blob],
      `clipboard-image-${files.length + 1}.${imageExtension(imageType)}`,
      { type: imageType },
    ));
  }
  return files;
}

async function nativeClipboardImage() {
  try {
    const { readImage } = await import("@tauri-apps/plugin-clipboard-manager");
    const image = await readImage();
    try {
      const [rgba, size] = await Promise.all([image.rgba(), image.size()]);
      if (!size.width || !size.height || rgba.length !== size.width * size.height * 4) {
        return [];
      }
      const canvas = document.createElement("canvas");
      canvas.width = size.width;
      canvas.height = size.height;
      const context = canvas.getContext("2d");
      if (!context) return [];
      context.putImageData(
        new ImageData(new Uint8ClampedArray(rgba), size.width, size.height),
        0,
        0,
      );
      const blob = await new Promise<Blob | null>((resolve) =>
        canvas.toBlob(resolve, "image/png")
      );
      return blob
        ? [new File([blob], "clipboard-image.png", { type: "image/png" })]
        : [];
    } finally {
      await image.close();
    }
  } catch {
    return [];
  }
}

export function clipboardHasImage(event: ClipboardEvent) {
  if (directImageFiles(event).length || clipboardImagePaths(event).length) return true;
  const types = [...(event.clipboardData?.types ?? [])];
  if (types.some((type) => type === "Files" || type.startsWith("image/"))) return true;
  return /<img[^>]+src=/i.test(event.clipboardData?.getData("text/html") ?? "");
}

export function clipboardHasFile(event: ClipboardEvent) {
  return directClipboardFiles(event).length > 0 || clipboardFilePaths(event).length > 0;
}

export function isImageAttachmentPath(path: string) {
  return /\.(gif|jpe?g|png|webp)$/i.test(path);
}

export function isImageAttachmentFile(file: File) {
  return file.type.startsWith("image/") || isImageAttachmentPath(file.name);
}

export async function prepareClipboardFile(
  file: File,
  index: number,
  language: "en" | "pt-BR",
): Promise<PromptAttachmentInput> {
  if (file.size > MAX_PASTED_FILE_BYTES) {
    throw new Error(message(
      language,
      "The pasted file exceeds the 25 MB limit",
      "O arquivo colado excede o limite de 25 MB",
    ));
  }
  const dataUrl = await readFileDataUrl(file, language);
  return {
    name: file.name || `clipboard-file-${index + 1}`,
    mimeType: file.type || "application/octet-stream",
    dataBase64: dataUrl.slice(dataUrl.indexOf(",") + 1),
    previewDataUrl: "",
  };
}

export function collectClipboardFiles(event: ClipboardEvent) {
  const files = directClipboardFiles(event);
  const paths = clipboardFilePaths(event);
  return { files, paths: files.length ? [] : paths };
}

export function clipboardMayContainImage(event: ClipboardEvent) {
  const clipboard = event.clipboardData;
  if (!clipboard) return true;
  const types = [...clipboard.types];
  if (!types.length) return true;
  const plainText = clipboard.getData("text/plain");
  const html = clipboard.getData("text/html");
  return !plainText && !html && types.every((type) =>
    type === "text/plain" || type === "text/html"
  );
}

export async function collectClipboardImages(
  event: ClipboardEvent,
  language: "en" | "pt-BR",
) {
  const paths = clipboardImagePaths(event);
  let files = directImageFiles(event);
  if (!files.length) files = dataUrlImageFiles(event);
  if (!files.length && !paths.length) {
    try {
      files = await asynchronousClipboardImages();
    } catch {
      // Some WebViews expose the image only through ClipboardEvent.
    }
  }
  if (!files.length && !paths.length) files = await nativeClipboardImage();
  if (!files.length && !paths.length) {
    throw new Error(message(
      language,
      "The clipboard did not expose readable image data",
      "A área de transferência não forneceu uma imagem legível",
    ));
  }
  return { files, paths: files.length ? [] : paths };
}
