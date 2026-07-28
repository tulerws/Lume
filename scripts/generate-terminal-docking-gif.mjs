import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { inflateSync } from "node:zlib";

const CARD_WIDTH = 420;
const CARD_HEIGHT = 340;
const MARGIN = 18;
const START_OFFSET = 100;
const WIDTH = MARGIN * 2 + CARD_WIDTH * 2 + START_OFFSET;
const HEIGHT = MARGIN * 2 + CARD_HEIGHT;
const FRAME_COUNT = 42;
const FRAME_DELAY = 7;

const codexPath = resolve("docs/media/lume-terminal-codex.png");
const claudePath = resolve("docs/media/lume-terminal-claude.png");
const outputPath = resolve(
  process.env.LUME_GIF_OUTPUT ?? "docs/media/lume-terminal-docking.gif",
);

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
  const signature = file.subarray(0, 8).toString("hex");
  if (signature !== "89504e470d0a1a0a") {
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

function clearCapturedHint(image, startY, endY) {
  for (let y = startY; y < endY; y += 1) {
    for (let x = 2; x < image.width - 2; x += 1) {
      const offset = (y * image.width + x) * 4;
      image.pixels[offset] = 20;
      image.pixels[offset + 1] = 29;
      image.pixels[offset + 2] = 25;
      image.pixels[offset + 3] = 255;
    }
  }
}

function clearCapturedArea(image, startX, startY, endX, endY) {
  for (let y = startY; y < endY; y += 1) {
    for (let x = startX; x < endX; x += 1) {
      const offset = (y * image.width + x) * 4;
      image.pixels[offset] = 20;
      image.pixels[offset + 1] = 29;
      image.pixels[offset + 2] = 25;
      image.pixels[offset + 3] = 255;
    }
  }
}

function createFrame() {
  const pixels = new Uint8Array(WIDTH * HEIGHT * 4);
  for (let offset = 0; offset < pixels.length; offset += 4) {
    pixels[offset] = 8;
    pixels[offset + 1] = 14;
    pixels[offset + 2] = 11;
    pixels[offset + 3] = 255;
  }
  return pixels;
}

function blendPixel(frame, x, y, red, green, blue, alpha = 255) {
  if (x < 0 || y < 0 || x >= WIDTH || y >= HEIGHT || alpha === 0) return;
  const offset = (y * WIDTH + x) * 4;
  const amount = alpha / 255;
  frame[offset] = Math.round(red * amount + frame[offset] * (1 - amount));
  frame[offset + 1] = Math.round(green * amount + frame[offset + 1] * (1 - amount));
  frame[offset + 2] = Math.round(blue * amount + frame[offset + 2] * (1 - amount));
}

function composite(frame, image, destinationX, destinationY) {
  for (let y = 0; y < image.height; y += 1) {
    for (let x = 0; x < image.width; x += 1) {
      const source = (y * image.width + x) * 4;
      let red = image.pixels[source];
      let green = image.pixels[source + 1];
      let blue = image.pixels[source + 2];
      const alpha = image.pixels[source + 3];

      // Firefox headless may paint its native scrollbar white. WebView uses the
      // app theme here, so normalize only that narrow capture artifact.
      if (
        x >= image.width - 13 &&
        y >= 58 &&
        red > 220 &&
        green > 220 &&
        blue > 220
      ) {
        red = 20;
        green = 29;
        blue = 25;
      }
      blendPixel(frame, destinationX + x, destinationY + y, red, green, blue, alpha);
    }
  }
}

function fillRect(frame, x, y, width, height, color, alpha = 255) {
  for (let row = Math.max(0, y); row < Math.min(HEIGHT, y + height); row += 1) {
    for (let column = Math.max(0, x); column < Math.min(WIDTH, x + width); column += 1) {
      blendPixel(frame, column, row, color[0], color[1], color[2], alpha);
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

function outlineRoundedRect(frame, x, y, width, height, radius, color, alpha = 255) {
  const inside = (column, row, inset) => {
    const left = x + inset;
    const top = y + inset;
    const right = x + width - inset - 1;
    const bottom = y + height - inset - 1;
    const innerRadius = Math.max(0, radius - inset);
    if (column < left || column > right || row < top || row > bottom) return false;
    if (
      (column >= left + innerRadius && column <= right - innerRadius) ||
      (row >= top + innerRadius && row <= bottom - innerRadius)
    ) {
      return true;
    }
    const centerX = column < left + innerRadius
      ? left + innerRadius
      : right - innerRadius;
    const centerY = row < top + innerRadius
      ? top + innerRadius
      : bottom - innerRadius;
    const dx = column - centerX;
    const dy = row - centerY;
    return dx * dx + dy * dy <= innerRadius * innerRadius;
  };

  for (let row = y; row < y + height; row += 1) {
    for (let column = x; column < x + width; column += 1) {
      if (inside(column, row, 0) && !inside(column, row, 2)) {
        blendPixel(frame, column, row, ...color, alpha);
      }
    }
  }
}

function drawDockHighlight(frame, targetX, targetY, pulse) {
  const green = pulse ? [94, 197, 148] : [76, 171, 126];
  outlineRoundedRect(frame, targetX, targetY, CARD_WIDTH, CARD_HEIGHT, 17, green, 190);
  const previewWidth = 126;
  const previewX = targetX + CARD_WIDTH - previewWidth - 12;
  const previewY = targetY + 42;
  const previewHeight = CARD_HEIGHT - 84;
  roundedRect(frame, previewX, previewY, previewWidth, previewHeight, 10, green, 54);
  outlineRoundedRect(
    frame,
    previewX,
    previewY,
    previewWidth,
    previewHeight,
    10,
    green,
    145,
  );
}

function drawMovingBorder(frame, x, y) {
  outlineRoundedRect(frame, x, y, CARD_WIDTH, CARD_HEIGHT, 17, [91, 186, 143], 150);
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

function drawCursor(frame, x, y, animationFrame) {
  const ringRadius = 6 + (animationFrame % 4);
  for (let angle = 0; angle < 360; angle += 5) {
    const radians = (angle * Math.PI) / 180;
    blendPixel(
      frame,
      Math.round(x + Math.cos(radians) * ringRadius),
      Math.round(y + Math.sin(radians) * ringRadius),
      91,
      173,
      220,
      180,
    );
  }

  const outline = [
    [x, y],
    [x, y + 25],
    [x + 6, y + 19],
    [x + 11, y + 29],
    [x + 16, y + 26],
    [x + 11, y + 17],
    [x + 21, y + 17],
  ];
  fillPolygon(
    frame,
    outline.map(([column, row]) => [column + 2, row + 2]),
    [0, 0, 0],
    110,
  );
  fillPolygon(frame, outline, [7, 12, 10], 255);
  fillPolygon(frame, [
    [x + 2, y + 4],
    [x + 2, y + 20],
    [x + 7, y + 15],
    [x + 12, y + 24],
    [x + 13, y + 23],
    [x + 8, y + 14],
    [x + 16, y + 14],
  ], [235, 242, 238], 255);
}

function drawRunningPulse(frame, x, y, animationFrame) {
  roundedRect(frame, x, y, 43, 27, 10, [30, 43, 37], 255);
  outlineRoundedRect(frame, x, y, 43, 27, 10, [62, 84, 74], 155);
  const active = animationFrame % 3;
  for (let index = 0; index < 3; index += 1) {
    const color = index === active ? [91, 168, 222] : [57, 101, 128];
    const offsetY = index === active ? -1 : 0;
    roundedRect(frame, x + 11 + index * 9, y + 12 + offsetY, 5, 5, 2, color, 255);
  }
}

function easeInOut(value) {
  return 0.5 - Math.cos(value * Math.PI) / 2;
}

function renderFrame(index, codex, claude) {
  const frame = createFrame();
  const firstX = MARGIN;
  const targetX = MARGIN + CARD_WIDTH;
  const initialX = targetX + START_OFFSET;
  const y = MARGIN;
  let secondX = initialX;
  let moving = false;
  let docked = false;

  if (index >= 8 && index <= 29) {
    moving = true;
    const progress = easeInOut((index - 8) / 21);
    secondX = Math.round(initialX + (targetX - initialX) * progress);
  } else if (index === 30) {
    moving = true;
    secondX = targetX - 6;
  } else if (index === 31) {
    moving = true;
    secondX = targetX + 3;
  } else if (index >= 32) {
    docked = true;
    secondX = targetX;
  }

  composite(frame, codex, firstX, y);
  composite(frame, claude, secondX, y);
  drawRunningPulse(frame, firstX + 12, y + 235, index);

  const previewActive = moving && secondX - targetX <= 62;
  if (previewActive) {
    drawDockHighlight(frame, firstX, y, index % 4 < 2);
    drawMovingBorder(frame, secondX, y);
  }

  if (moving) {
    drawCursor(frame, secondX + 292, y + 17, index);
  }

  if (docked && index < 38) {
    const fade = Math.max(0, 170 - (index - 32) * 28);
    outlineRoundedRect(frame, firstX, y, CARD_WIDTH * 2, CARD_HEIGHT, 17, [91, 186, 143], fade);
  }

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
      return { channel, range: maximum - minimum };
    });
    const widest = ranges.sort((left, right) => right.range - left.range)[0];
    const total = boxColors.reduce((sum, color) => sum + color.count, 0);
    return {
      colors: boxColors,
      channel: widest.channel,
      score: widest.range * Math.log2(total + 1),
      total,
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

  const paletteColors = boxes.map((box) => {
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
  while (paletteColors.length < 256) paletteColors.push([0, 0, 0]);
  return paletteColors.slice(0, 256);
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

const codex = decodePng(codexPath);
const claude = decodePng(claudePath);
clearCapturedHint(codex, 263, 281);
clearCapturedHint(claude, 234, 258);
clearCapturedArea(codex, 9, 230, 58, 265);
if (
  codex.width !== CARD_WIDTH ||
  codex.height !== CARD_HEIGHT ||
  claude.width !== CARD_WIDTH ||
  claude.height !== CARD_HEIGHT
) {
  throw new Error(`Terminal captures must be ${CARD_WIDTH}x${CARD_HEIGHT}`);
}

const requestedFrame = Number(process.env.LUME_GIF_PREVIEW_FRAME);
const frameIndexes = Number.isInteger(requestedFrame) &&
    requestedFrame >= 0 &&
    requestedFrame < FRAME_COUNT
  ? [requestedFrame]
  : Array.from({ length: FRAME_COUNT }, (_, index) => index);
const frames = frameIndexes.map((index) => renderFrame(index, codex, claude));
writeFileSync(outputPath, encodeGif(frames));
console.log(
  `Generated ${outputPath} (${WIDTH}x${HEIGHT}, ${frames.length} frames, ${(frames.length * FRAME_DELAY) / 100}s)`,
);
