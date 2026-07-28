import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { inflateSync } from "node:zlib";

const WIDTH = 900;
const HEIGHT = 500;
const PANEL_WIDTH = 420;
const PANEL_HEIGHT = 408;
const ORB_WIDTH = 78;
const ORB_HEIGHT = 44;
const PANEL_X = 452;
const PANEL_Y = 58;
const FRAME_COUNT = 52;
const FRAME_DELAY = 7;
const OPENING_FRAMES = 9;

const orbPath = resolve("docs/media/lume-orb-running.png");
const panelPath = resolve("docs/screenshots/lume-desktop-sessions.png");
const outputPath = resolve(
  process.env.LUME_ORB_GIF_OUTPUT ?? "docs/media/lume-orb-panel.gif",
);
const previewFrame = process.env.LUME_ORB_GIF_PREVIEW_FRAME
  ? Number(process.env.LUME_ORB_GIF_PREVIEW_FRAME)
  : null;

function readUInt32(buffer, offset) {
  return (
    buffer[offset] * 0x1000000 +
    buffer[offset + 1] * 0x10000 +
    buffer[offset + 2] * 0x100 +
    buffer[offset + 3]
  );
}

function paeth(left, above, upperLeft) {
  const prediction = left + above - upperLeft;
  const leftDistance = Math.abs(prediction - left);
  const aboveDistance = Math.abs(prediction - above);
  const upperLeftDistance = Math.abs(prediction - upperLeft);
  if (leftDistance <= aboveDistance && leftDistance <= upperLeftDistance) return left;
  if (aboveDistance <= upperLeftDistance) return above;
  return upperLeft;
}

function decodePng(path) {
  const file = readFileSync(path);
  if (file.subarray(0, 8).toString("hex") !== "89504e470d0a1a0a") {
    throw new Error(`${path} is not a PNG file`);
  }

  let width = 0;
  let height = 0;
  let bitDepth = 0;
  let colorType = 0;
  const compressed = [];

  for (let offset = 8; offset < file.length;) {
    const length = readUInt32(file, offset);
    const type = file.subarray(offset + 4, offset + 8).toString("ascii");
    const data = file.subarray(offset + 8, offset + 8 + length);
    if (type === "IHDR") {
      width = readUInt32(data, 0);
      height = readUInt32(data, 4);
      bitDepth = data[8];
      colorType = data[9];
    } else if (type === "IDAT") {
      compressed.push(data);
    } else if (type === "IEND") {
      break;
    }
    offset += length + 12;
  }

  if (bitDepth !== 8 || ![2, 6].includes(colorType)) {
    throw new Error(`${path} must be an 8-bit RGB or RGBA PNG`);
  }

  const channels = colorType === 6 ? 4 : 3;
  const scanlineLength = width * channels;
  const inflated = inflateSync(Buffer.concat(compressed));
  const pixels = new Uint8Array(width * height * 4);
  let sourceOffset = 0;
  let previous = new Uint8Array(scanlineLength);

  for (let y = 0; y < height; y += 1) {
    const filter = inflated[sourceOffset];
    sourceOffset += 1;
    const raw = inflated.subarray(sourceOffset, sourceOffset + scanlineLength);
    sourceOffset += scanlineLength;
    const row = new Uint8Array(scanlineLength);

    for (let x = 0; x < scanlineLength; x += 1) {
      const left = x >= channels ? row[x - channels] : 0;
      const above = previous[x] ?? 0;
      const upperLeft = x >= channels ? previous[x - channels] : 0;
      const value = raw[x];
      row[x] = {
        0: value,
        1: value + left,
        2: value + above,
        3: value + Math.floor((left + above) / 2),
        4: value + paeth(left, above, upperLeft),
      }[filter] & 0xff;
    }

    for (let x = 0; x < width; x += 1) {
      const source = x * channels;
      const target = (y * width + x) * 4;
      pixels[target] = row[source];
      pixels[target + 1] = row[source + 1];
      pixels[target + 2] = row[source + 2];
      pixels[target + 3] = channels === 4 ? row[source + 3] : 255;
    }
    previous = row;
  }

  return { width, height, pixels };
}

function blendPixel(frame, x, y, red, green, blue, alpha = 255) {
  if (x < 0 || y < 0 || x >= WIDTH || y >= HEIGHT || alpha === 0) return;
  const offset = (y * WIDTH + x) * 4;
  const amount = alpha / 255;
  frame[offset] = Math.round(red * amount + frame[offset] * (1 - amount));
  frame[offset + 1] = Math.round(green * amount + frame[offset + 1] * (1 - amount));
  frame[offset + 2] = Math.round(blue * amount + frame[offset + 2] * (1 - amount));
}

function fillRect(frame, x, y, width, height, color, alpha = 255) {
  for (let row = Math.max(0, y); row < Math.min(HEIGHT, y + height); row += 1) {
    for (let column = Math.max(0, x); column < Math.min(WIDTH, x + width); column += 1) {
      blendPixel(frame, column, row, ...color, alpha);
    }
  }
}

function roundedRect(frame, x, y, width, height, radius, color, alpha = 255) {
  fillRect(frame, x + radius, y, width - radius * 2, height, color, alpha);
  fillRect(frame, x, y + radius, width, height - radius * 2, color, alpha);
  for (let row = 0; row < radius; row += 1) {
    for (let column = 0; column < radius; column += 1) {
      const dx = radius - column - 0.5;
      const dy = radius - row - 0.5;
      if (dx * dx + dy * dy <= radius * radius) {
        blendPixel(frame, x + column, y + row, ...color, alpha);
        blendPixel(frame, x + width - column - 1, y + row, ...color, alpha);
        blendPixel(frame, x + column, y + height - row - 1, ...color, alpha);
        blendPixel(frame, x + width - column - 1, y + height - row - 1, ...color, alpha);
      }
    }
  }
}

function fillPolygon(frame, points, color, alpha = 255) {
  const minimumX = Math.floor(Math.min(...points.map(([x]) => x)));
  const maximumX = Math.ceil(Math.max(...points.map(([x]) => x)));
  const minimumY = Math.floor(Math.min(...points.map(([, y]) => y)));
  const maximumY = Math.ceil(Math.max(...points.map(([, y]) => y)));
  for (let y = minimumY; y <= maximumY; y += 1) {
    for (let x = minimumX; x <= maximumX; x += 1) {
      let inside = false;
      for (
        let current = 0, previous = points.length - 1;
        current < points.length;
        previous = current++
      ) {
        const [currentX, currentY] = points[current];
        const [previousX, previousY] = points[previous];
        const crosses = currentY > y !== previousY > y &&
          x < ((previousX - currentX) * (y - currentY)) /
            (previousY - currentY) + currentX;
        if (crosses) inside = !inside;
      }
      if (inside) blendPixel(frame, x, y, ...color, alpha);
    }
  }
}

function drawCircle(frame, centerX, centerY, radius, color, alpha = 255) {
  const radiusSquared = radius * radius;
  for (let y = centerY - radius; y <= centerY + radius; y += 1) {
    for (let x = centerX - radius; x <= centerX + radius; x += 1) {
      const dx = x - centerX;
      const dy = y - centerY;
      if (dx * dx + dy * dy <= radiusSquared) {
        blendPixel(frame, x, y, ...color, alpha);
      }
    }
  }
}

function createDesktop() {
  const frame = new Uint8Array(WIDTH * HEIGHT * 4);
  for (let y = 0; y < HEIGHT; y += 1) {
    const vertical = y / HEIGHT;
    for (let x = 0; x < WIDTH; x += 1) {
      const horizontal = x / WIDTH;
      const offset = (y * WIDTH + x) * 4;
      frame[offset] = Math.round(25 + 14 * horizontal + 8 * vertical);
      frame[offset + 1] = Math.round(47 + 21 * horizontal + 13 * vertical);
      frame[offset + 2] = Math.round(53 + 20 * horizontal + 16 * vertical);
      frame[offset + 3] = 255;
    }
  }

  drawCircle(frame, 178, 390, 260, [43, 91, 94], 145);
  drawCircle(frame, 318, 92, 190, [74, 117, 112], 74);
  fillPolygon(frame, [[0, 364], [226, 112], [408, 500], [0, 500]], [19, 35, 41], 95);
  fillPolygon(frame, [[270, 500], [518, 146], [678, 500]], [89, 131, 121], 42);
  fillRect(frame, 0, 0, WIDTH, 30, [13, 19, 21], 214);
  roundedRect(frame, 18, 9, 47, 4, 2, [139, 162, 157], 115);
  roundedRect(frame, WIDTH - 80, 9, 16, 4, 2, [139, 162, 157], 115);
  roundedRect(frame, WIDTH - 54, 8, 7, 7, 3, [139, 162, 157], 115);
  roundedRect(frame, WIDTH - 36, 7, 12, 9, 2, [139, 162, 157], 115);
  return frame;
}

function insideRoundedRect(x, y, width, height, radius) {
  if (x < 0 || y < 0 || x >= width || y >= height) return false;
  const horizontalCenter = x >= radius && x < width - radius;
  const verticalCenter = y >= radius && y < height - radius;
  if (horizontalCenter || verticalCenter) return true;
  const centerX = x < radius ? radius : width - radius - 1;
  const centerY = y < radius ? radius : height - radius - 1;
  const dx = x - centerX;
  const dy = y - centerY;
  return dx * dx + dy * dy <= radius * radius;
}

function animateMascot(image, region, backgroundSample, offsetY) {
  if (offsetY === 0) return image;
  const pixels = new Uint8Array(image.pixels);
  const sampleOffset =
    (backgroundSample.y * image.width + backgroundSample.x) * 4;
  const background = [
    image.pixels[sampleOffset],
    image.pixels[sampleOffset + 1],
    image.pixels[sampleOffset + 2],
    image.pixels[sampleOffset + 3],
  ];
  for (let y = region.y; y < region.y + region.height; y += 1) {
    for (let x = region.x; x < region.x + region.width; x += 1) {
      const source = (y * image.width + x) * 4;
      pixels.set(background, source);
    }
  }

  for (let y = region.y; y < region.y + region.height; y += 1) {
    for (let x = region.x; x < region.x + region.width; x += 1) {
      const destinationY = y + offsetY;
      if (destinationY < 0 || destinationY >= image.height) continue;
      const source = (y * image.width + x) * 4;
      const destination = (destinationY * image.width + x) * 4;
      pixels[destination] = image.pixels[source];
      pixels[destination + 1] = image.pixels[source + 1];
      pixels[destination + 2] = image.pixels[source + 2];
      pixels[destination + 3] = image.pixels[source + 3];
    }
  }
  return { ...image, pixels };
}

function compositePanel(frame, panel, width, height, radius, alpha) {
  const visibleWidth = Math.min(width, panel.width);
  const visibleHeight = Math.min(height, panel.height);
  for (let y = 0; y < visibleHeight; y += 1) {
    for (let x = 0; x < visibleWidth; x += 1) {
      if (!insideRoundedRect(x, y, width, height, radius)) continue;
      const source = (y * panel.width + x) * 4;
      blendPixel(
        frame,
        PANEL_X + x,
        PANEL_Y + y,
        panel.pixels[source],
        panel.pixels[source + 1],
        panel.pixels[source + 2],
        Math.round((panel.pixels[source + 3] * alpha) / 255),
      );
    }
  }
}

function drawCursor(frame, x, y) {
  const pointer = [
    [x, y],
    [x, y + 27],
    [x + 7, y + 20],
    [x + 12, y + 31],
    [x + 18, y + 28],
    [x + 13, y + 18],
    [x + 24, y + 18],
  ];
  fillPolygon(
    frame,
    pointer.map(([column, row]) => [column + 2, row + 3]),
    [0, 0, 0],
    125,
  );
  fillPolygon(frame, pointer, [24, 31, 30], 255);
  fillPolygon(frame, [
    [x + 2, y + 4],
    [x + 2, y + 22],
    [x + 8, y + 16],
    [x + 13, y + 26],
    [x + 15, y + 25],
    [x + 10, y + 15],
    [x + 19, y + 15],
  ], [245, 249, 247], 255);
}

function drawClickPulse(frame, x, y, progress) {
  const radius = Math.round(5 + progress * 13);
  const alpha = Math.round(210 * (1 - progress));
  for (let angle = 0; angle < 360; angle += 3) {
    const radians = (angle * Math.PI) / 180;
    blendPixel(
      frame,
      Math.round(x + Math.cos(radians) * radius),
      Math.round(y + Math.sin(radians) * radius),
      96,
      177,
      224,
      alpha,
    );
  }
}

function easeInOut(value) {
  return 0.5 - Math.cos(Math.max(0, Math.min(1, value)) * Math.PI) / 2;
}

function renderFrame(index, desktop, orbVariants, panelVariants) {
  const frame = new Uint8Array(desktop);
  const animationStep = Math.floor(index / 3) % 2;
  const orb = orbVariants[animationStep];
  const panel = panelVariants[animationStep];
  const orbCenterX = PANEL_X + ORB_WIDTH / 2;
  const orbCenterY = PANEL_Y + ORB_HEIGHT / 2;
  const cursorStart = { x: 154, y: 318 };
  const cursorTarget = { x: orbCenterX - 3, y: orbCenterY - 2 };

  if (index < 26) {
    compositePanel(frame, orb, ORB_WIDTH, ORB_HEIGHT, 22, 255);
  }

  let cursorX = cursorStart.x;
  let cursorY = cursorStart.y;
  if (index >= 9) {
    const progress = easeInOut((index - 9) / 13);
    cursorX = Math.round(cursorStart.x + (cursorTarget.x - cursorStart.x) * progress);
    cursorY = Math.round(cursorStart.y + (cursorTarget.y - cursorStart.y) * progress);
  }

  if (index >= 23 && index <= 27) {
    drawClickPulse(frame, orbCenterX, orbCenterY, (index - 23) / 4);
  }

  if (index >= 26) {
    const progress = easeInOut((index - 26) / OPENING_FRAMES);
    const width = Math.round(ORB_WIDTH + (PANEL_WIDTH - ORB_WIDTH) * progress);
    const height = Math.round(ORB_HEIGHT + (PANEL_HEIGHT - ORB_HEIGHT) * progress);
    const radius = Math.round(22 - progress * 2);
    roundedRect(frame, PANEL_X, PANEL_Y, width, height, radius, [24, 34, 30], 255);
    compositePanel(frame, panel, width, height, radius, Math.round(105 + progress * 150));
    cursorX = Math.round(cursorTarget.x + 18 * progress);
    cursorY = Math.round(cursorTarget.y + 22 * progress);
  }

  drawCursor(frame, cursorX, cursorY);
  return frame;
}

function buildPalette(frames) {
  const histogram = new Map();
  for (const frame of frames) {
    for (let offset = 0; offset < frame.length; offset += 4) {
      const red = frame[offset];
      const green = frame[offset + 1];
      const blue = frame[offset + 2];
      const key = ((red >> 3) << 10) | ((green >> 3) << 5) | (blue >> 3);
      const entry = histogram.get(key);
      if (entry) {
        entry.count += 1;
        entry.red += red;
        entry.green += green;
        entry.blue += blue;
      } else {
        histogram.set(key, { count: 1, red, green, blue });
      }
    }
  }

  const colors = Array.from(histogram.values(), (entry) => ({
    count: entry.count,
    red: entry.red / entry.count,
    green: entry.green / entry.count,
    blue: entry.blue / entry.count,
  }));
  const describe = (boxColors) => {
    const ranges = ["red", "green", "blue"].map((channel) => {
      let minimum = 255;
      let maximum = 0;
      for (const color of boxColors) {
        minimum = Math.min(minimum, color[channel]);
        maximum = Math.max(maximum, color[channel]);
      }
      const total = boxColors.reduce((sum, color) => sum + color.count, 0);
      return { channel, range: maximum - minimum, total };
    });
    const widest = ranges.sort((left, right) => right.range - left.range)[0];
    return {
      colors: boxColors,
      channel: widest.channel,
      score: widest.range * Math.log2(widest.total + 1),
      total: widest.total,
    };
  };

  const boxes = [describe(colors)];
  while (boxes.length < 256) {
    boxes.sort((left, right) => right.score - left.score);
    const box = boxes.shift();
    if (!box || box.colors.length < 2) {
      if (box) boxes.push(box);
      break;
    }
    box.colors.sort((left, right) => left[box.channel] - right[box.channel]);
    const halfway = box.total / 2;
    let accumulated = 0;
    let split = 1;
    for (; split < box.colors.length; split += 1) {
      accumulated += box.colors[split - 1].count;
      if (accumulated >= halfway) break;
    }
    split = Math.max(1, Math.min(box.colors.length - 1, split));
    boxes.push(
      describe(box.colors.slice(0, split)),
      describe(box.colors.slice(split)),
    );
  }

  const palette = boxes.map((box) => {
    const totals = box.colors.reduce(
      (result, color) => {
        result.red += color.red * color.count;
        result.green += color.green * color.count;
        result.blue += color.blue * color.count;
        result.count += color.count;
        return result;
      },
      { red: 0, green: 0, blue: 0, count: 0 },
    );
    return [
      Math.round(totals.red / totals.count),
      Math.round(totals.green / totals.count),
      Math.round(totals.blue / totals.count),
    ];
  });
  while (palette.length < 256) palette.push([0, 0, 0]);
  return palette.slice(0, 256);
}

function quantize(frame, palette) {
  const indexed = new Uint8Array(WIDTH * HEIGHT);
  const cache = new Uint16Array(32 * 32 * 32).fill(0xffff);
  for (let source = 0, target = 0; source < frame.length; source += 4, target += 1) {
    const red = frame[source];
    const green = frame[source + 1];
    const blue = frame[source + 2];
    const key = ((red >> 3) << 10) | ((green >> 3) << 5) | (blue >> 3);
    if (cache[key] === 0xffff) {
      let bestIndex = 0;
      let bestDistance = Number.POSITIVE_INFINITY;
      for (let index = 0; index < palette.length; index += 1) {
        const color = palette[index];
        const redDistance = red - color[0];
        const greenDistance = green - color[1];
        const blueDistance = blue - color[2];
        const distance =
          redDistance * redDistance +
          greenDistance * greenDistance +
          blueDistance * blueDistance;
        if (distance < bestDistance) {
          bestDistance = distance;
          bestIndex = index;
        }
      }
      cache[key] = bestIndex;
    }
    indexed[target] = cache[key];
  }
  return indexed;
}

function pushWord(bytes, value) {
  bytes.push(value & 0xff, (value >> 8) & 0xff);
}

function compressLzw(indices) {
  const clearCode = 256;
  const endCode = 257;
  const bytes = [];
  let current = 0;
  let bitCount = 0;
  let codeSize = 9;
  let nextCode = 258;
  let dictionary = new Map();

  const writeCode = (code, size = codeSize) => {
    current |= code << bitCount;
    bitCount += size;
    while (bitCount >= 8) {
      bytes.push(current & 0xff);
      current >>= 8;
      bitCount -= 8;
    }
  };
  const resetDictionary = () => {
    dictionary = new Map();
    codeSize = 9;
    nextCode = 258;
  };

  writeCode(clearCode);
  let prefix = indices[0];
  for (let index = 1; index < indices.length; index += 1) {
    const symbol = indices[index];
    const key = (prefix << 8) | symbol;
    const existing = dictionary.get(key);
    if (existing !== undefined) {
      prefix = existing;
      continue;
    }
    writeCode(prefix);
    if (nextCode < 4096) {
      dictionary.set(key, nextCode);
      nextCode += 1;
      if (nextCode > 1 << codeSize && codeSize < 12) codeSize += 1;
    } else {
      writeCode(clearCode);
      resetDictionary();
    }
    prefix = symbol;
  }
  writeCode(prefix);
  writeCode(endCode);
  if (bitCount > 0) bytes.push(current & 0xff);
  return bytes;
}

function encodeGif(frames) {
  const palette = buildPalette(frames);
  const bytes = [...Buffer.from("GIF89a", "ascii")];
  pushWord(bytes, WIDTH);
  pushWord(bytes, HEIGHT);
  bytes.push(0xf7, 0, 0, ...palette.flat());
  bytes.push(
    0x21, 0xff, 0x0b,
    ...Buffer.from("NETSCAPE2.0", "ascii"),
    0x03, 0x01, 0x00, 0x00, 0x00,
  );

  for (const frame of frames) {
    bytes.push(0x21, 0xf9, 0x04, 0x04);
    pushWord(bytes, FRAME_DELAY);
    bytes.push(0x00, 0x00, 0x2c);
    pushWord(bytes, 0);
    pushWord(bytes, 0);
    pushWord(bytes, WIDTH);
    pushWord(bytes, HEIGHT);
    bytes.push(0x00, 0x08);
    const compressed = compressLzw(quantize(frame, palette));
    for (let offset = 0; offset < compressed.length; offset += 255) {
      const block = compressed.slice(offset, offset + 255);
      bytes.push(block.length, ...block);
    }
    bytes.push(0x00);
  }
  bytes.push(0x3b);
  return Buffer.from(bytes);
}

const orb = decodePng(orbPath);
const panel = decodePng(panelPath);
if (orb.width !== ORB_WIDTH || orb.height !== ORB_HEIGHT) {
  throw new Error(`Expected a ${ORB_WIDTH}x${ORB_HEIGHT} orb capture`);
}
if (panel.width !== PANEL_WIDTH || panel.height !== PANEL_HEIGHT) {
  throw new Error(`Expected a ${PANEL_WIDTH}x${PANEL_HEIGHT} panel capture`);
}

const desktop = createDesktop();
const orbVariants = [
  orb,
  animateMascot(
    orb,
    { x: 8, y: 6, width: 36, height: 32 },
    { x: 45, y: 22 },
    -2,
  ),
];
const panelVariants = [
  panel,
  animateMascot(
    panel,
    { x: 12, y: 7, width: 38, height: 34 },
    { x: 52, y: 24 },
    -2,
  ),
];
const requestedFrames = previewFrame === null
  ? Array.from({ length: FRAME_COUNT }, (_, index) => index)
  : [Math.max(0, Math.min(FRAME_COUNT - 1, previewFrame))];
const frames = requestedFrames.map((index) =>
  renderFrame(index, desktop, orbVariants, panelVariants)
);
writeFileSync(outputPath, encodeGif(frames));
console.log(`Generated ${outputPath} with ${frames.length} frame(s).`);
