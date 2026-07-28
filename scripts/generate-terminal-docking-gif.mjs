import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";

const WIDTH = 840;
const HEIGHT = 400;
const FRAME_COUNT = 36;
const FRAME_DELAY = 12;
const outputPath = resolve("docs/media/lume-terminal-docking.gif");

const palette = [
  [10, 16, 13],
  [20, 32, 27],
  [47, 70, 59],
  [220, 232, 225],
  [130, 153, 142],
  [82, 198, 137],
  [84, 167, 247],
  [226, 176, 87],
];

const C = {
  background: 0,
  surface: 1,
  border: 2,
  text: 3,
  muted: 4,
  green: 5,
  blue: 6,
  amber: 7,
};

const FONT = {
  " ": ["00000", "00000", "00000", "00000", "00000", "00000", "00000"],
  A: ["01110", "10001", "10001", "11111", "10001", "10001", "10001"],
  B: ["11110", "10001", "10001", "11110", "10001", "10001", "11110"],
  C: ["01111", "10000", "10000", "10000", "10000", "10000", "01111"],
  D: ["11110", "10001", "10001", "10001", "10001", "10001", "11110"],
  E: ["11111", "10000", "10000", "11110", "10000", "10000", "11111"],
  F: ["11111", "10000", "10000", "11110", "10000", "10000", "10000"],
  G: ["01111", "10000", "10000", "10111", "10001", "10001", "01111"],
  H: ["10001", "10001", "10001", "11111", "10001", "10001", "10001"],
  I: ["11111", "00100", "00100", "00100", "00100", "00100", "11111"],
  J: ["00111", "00010", "00010", "00010", "10010", "10010", "01100"],
  K: ["10001", "10010", "10100", "11000", "10100", "10010", "10001"],
  L: ["10000", "10000", "10000", "10000", "10000", "10000", "11111"],
  M: ["10001", "11011", "10101", "10101", "10001", "10001", "10001"],
  N: ["10001", "11001", "10101", "10011", "10001", "10001", "10001"],
  O: ["01110", "10001", "10001", "10001", "10001", "10001", "01110"],
  P: ["11110", "10001", "10001", "11110", "10000", "10000", "10000"],
  Q: ["01110", "10001", "10001", "10001", "10101", "10010", "01101"],
  R: ["11110", "10001", "10001", "11110", "10100", "10010", "10001"],
  S: ["01111", "10000", "10000", "01110", "00001", "00001", "11110"],
  T: ["11111", "00100", "00100", "00100", "00100", "00100", "00100"],
  U: ["10001", "10001", "10001", "10001", "10001", "10001", "01110"],
  V: ["10001", "10001", "10001", "10001", "10001", "01010", "00100"],
  W: ["10001", "10001", "10001", "10101", "10101", "10101", "01010"],
  X: ["10001", "10001", "01010", "00100", "01010", "10001", "10001"],
  Y: ["10001", "10001", "01010", "00100", "00100", "00100", "00100"],
  Z: ["11111", "00001", "00010", "00100", "01000", "10000", "11111"],
  "0": ["01110", "10001", "10011", "10101", "11001", "10001", "01110"],
  "1": ["00100", "01100", "00100", "00100", "00100", "00100", "01110"],
  "2": ["01110", "10001", "00001", "00010", "00100", "01000", "11111"],
  "3": ["11110", "00001", "00001", "01110", "00001", "00001", "11110"],
  "4": ["00010", "00110", "01010", "10010", "11111", "00010", "00010"],
  "5": ["11111", "10000", "10000", "11110", "00001", "00001", "11110"],
  "6": ["01110", "10000", "10000", "11110", "10001", "10001", "01110"],
  "7": ["11111", "00001", "00010", "00100", "01000", "01000", "01000"],
  "8": ["01110", "10001", "10001", "01110", "10001", "10001", "01110"],
  "9": ["01110", "10001", "10001", "01111", "00001", "00001", "01110"],
  ".": ["00000", "00000", "00000", "00000", "00000", "00110", "00110"],
  ":": ["00000", "00110", "00110", "00000", "00110", "00110", "00000"],
  "-": ["00000", "00000", "00000", "11111", "00000", "00000", "00000"],
  "/": ["00001", "00010", "00010", "00100", "01000", "01000", "10000"],
  ">": ["10000", "01000", "00100", "00010", "00100", "01000", "10000"],
  "$": ["00100", "01111", "10100", "01110", "00101", "11110", "00100"],
  "+": ["00000", "00100", "00100", "11111", "00100", "00100", "00000"],
};

function frameBuffer() {
  return new Uint8Array(WIDTH * HEIGHT).fill(C.background);
}

function setPixel(buffer, x, y, color) {
  if (x >= 0 && y >= 0 && x < WIDTH && y < HEIGHT) {
    buffer[y * WIDTH + x] = color;
  }
}

function fillRect(buffer, x, y, width, height, color) {
  const startX = Math.max(0, Math.round(x));
  const startY = Math.max(0, Math.round(y));
  const endX = Math.min(WIDTH, Math.round(x + width));
  const endY = Math.min(HEIGHT, Math.round(y + height));
  for (let row = startY; row < endY; row += 1) {
    buffer.fill(color, row * WIDTH + startX, row * WIDTH + endX);
  }
}

function roundedRect(buffer, x, y, width, height, radius, color) {
  fillRect(buffer, x + radius, y, width - radius * 2, height, color);
  fillRect(buffer, x, y + radius, width, height - radius * 2, color);
  for (let dy = 0; dy < radius; dy += 1) {
    for (let dx = 0; dx < radius; dx += 1) {
      const distance = (radius - dx - 0.5) ** 2 + (radius - dy - 0.5) ** 2;
      if (distance <= radius ** 2) {
        setPixel(buffer, x + dx, y + dy, color);
        setPixel(buffer, x + width - dx - 1, y + dy, color);
        setPixel(buffer, x + dx, y + height - dy - 1, color);
        setPixel(buffer, x + width - dx - 1, y + height - dy - 1, color);
      }
    }
  }
}

function outlineRoundedRect(buffer, x, y, width, height, radius, color, thickness = 2) {
  roundedRect(buffer, x, y, width, height, radius, color);
  roundedRect(
    buffer,
    x + thickness,
    y + thickness,
    width - thickness * 2,
    height - thickness * 2,
    Math.max(1, radius - thickness),
    C.surface,
  );
}

function drawLine(buffer, x1, y1, x2, y2, color) {
  let x = Math.round(x1);
  let y = Math.round(y1);
  const targetX = Math.round(x2);
  const targetY = Math.round(y2);
  const dx = Math.abs(targetX - x);
  const sx = x < targetX ? 1 : -1;
  const dy = -Math.abs(targetY - y);
  const sy = y < targetY ? 1 : -1;
  let error = dx + dy;
  while (true) {
    setPixel(buffer, x, y, color);
    if (x === targetX && y === targetY) break;
    const doubled = error * 2;
    if (doubled >= dy) {
      error += dy;
      x += sx;
    }
    if (doubled <= dx) {
      error += dx;
      y += sy;
    }
  }
}

function drawText(buffer, text, x, y, color, scale = 1) {
  let cursor = Math.round(x);
  for (const rawCharacter of String(text).toUpperCase()) {
    const glyph = FONT[rawCharacter] ?? FONT[" "];
    for (let row = 0; row < 7; row += 1) {
      for (let column = 0; column < 5; column += 1) {
        if (glyph[row][column] === "1") {
          fillRect(
            buffer,
            cursor + column * scale,
            y + row * scale,
            scale,
            scale,
            color,
          );
        }
      }
    }
    cursor += 6 * scale;
  }
}

function textWidth(text, scale = 1) {
  return Math.max(0, String(text).length * 6 * scale - scale);
}

function drawBadge(buffer, text, right, y, color) {
  const width = textWidth(text) + 13;
  roundedRect(buffer, right - width, y, width, 17, 7, C.border);
  roundedRect(buffer, right - width + 1, y + 1, width - 2, 15, 6, C.surface);
  drawText(buffer, text, right - width + 7, y + 5, color);
}

function drawMascot(buffer, x, y, awake) {
  const color = awake ? C.green : C.muted;
  const pixels = [
    "00111100",
    "01111110",
    "11100110",
    "11111110",
    "11111000",
    "01111100",
    "00110110",
    "00110010",
  ];
  for (let row = 0; row < pixels.length; row += 1) {
    for (let column = 0; column < pixels[row].length; column += 1) {
      if (pixels[row][column] === "1") {
        fillRect(buffer, x + column * 2, y + row * 2, 2, 2, color);
      }
    }
  }
  fillRect(buffer, x + 11, y + 4, 2, 2, C.background);
}

function drawTerminal(buffer, {
  x,
  y,
  width,
  height,
  agent,
  project,
  source,
  running,
  moving,
  docked,
  frame,
}) {
  const borderColor = moving ? C.amber : docked ? C.green : C.border;
  roundedRect(buffer, x + 5, y + 7, width, height, 15, C.background);
  outlineRoundedRect(buffer, x, y, width, height, 15, borderColor, 2);
  fillRect(buffer, x + 2, y + 49, width - 4, 1, C.border);

  roundedRect(buffer, x + 14, y + 12, 28, 28, 8, C.border);
  roundedRect(buffer, x + 16, y + 14, 24, 24, 7, C.surface);
  drawMascot(buffer, x + 20, y + 18, running);
  drawText(buffer, agent, x + 51, y + 14, C.text, 2);
  drawText(buffer, project, x + 52, y + 32, C.muted);
  drawBadge(buffer, source, x + width - 14, y + 16, source === "CLI" ? C.muted : C.blue);

  drawText(buffer, "$", x + 18, y + 67, C.green);
  drawText(buffer, project, x + 31, y + 67, C.muted);
  drawText(buffer, ">", x + 18, y + 88, running ? C.blue : C.amber);
  drawText(buffer, running ? "RUNNING" : "WAITING FOR INPUT", x + 31, y + 88, running ? C.blue : C.amber);
  if (running) {
    const activeDot = frame % 3;
    for (let dot = 0; dot < 3; dot += 1) {
      fillRect(buffer, x + 81 + dot * 6, y + 88, 3, 3, dot === activeDot ? C.blue : C.border);
    }
  }

  roundedRect(buffer, x + 15, y + 112, width - 30, 72, 9, C.border);
  roundedRect(buffer, x + 17, y + 114, width - 34, 68, 8, C.surface);
  drawText(buffer, running ? "CURRENT STEP" : "LAST RESPONSE", x + 27, y + 124, C.muted);
  drawText(buffer, running ? "UPDATING SESSION MONITOR" : "READY FOR THE NEXT PROMPT", x + 27, y + 143, C.text);
  drawText(buffer, running ? "+ TERMINAL WINDOWS" : "NO PENDING ACTIONS", x + 27, y + 160, running ? C.green : C.muted);

  roundedRect(buffer, x + 15, y + height - 54, width - 65, 37, 9, C.border);
  roundedRect(buffer, x + 17, y + height - 52, width - 69, 33, 8, C.surface);
  drawText(buffer, running ? "AGENT IS WORKING" : "PROMPT FOR AGENT", x + 29, y + height - 40, C.muted);
  roundedRect(buffer, x + width - 44, y + height - 54, 29, 37, 9, running ? C.border : C.green);
  drawText(buffer, running ? "..." : ">", x + width - 37, y + height - 41, running ? C.muted : C.text);
}

function drawDashedPreview(buffer, x, y, width, height, pulse) {
  const color = pulse ? C.amber : C.green;
  for (let offset = 0; offset < width; offset += 12) {
    fillRect(buffer, x + offset, y, 7, 2, color);
    fillRect(buffer, x + offset, y + height - 2, 7, 2, color);
  }
  for (let offset = 0; offset < height; offset += 12) {
    fillRect(buffer, x, y + offset, 2, 7, color);
    fillRect(buffer, x + width - 2, y + offset, 2, 7, color);
  }
  const label = "RELEASE TO DOCK";
  const labelWidth = textWidth(label) + 18;
  roundedRect(buffer, x + (width - labelWidth) / 2, y - 28, labelWidth, 20, 9, C.border);
  drawText(buffer, label, x + (width - labelWidth) / 2 + 9, y - 21, color);
}

function drawCursor(buffer, x, y) {
  for (let row = 0; row < 18; row += 1) {
    for (let column = 0; column <= Math.floor(row * 0.55); column += 1) {
      setPixel(buffer, x + column, y + row, C.text);
    }
  }
  drawLine(buffer, x + 7, y + 12, x + 15, y + 20, C.text);
  drawLine(buffer, x + 8, y + 12, x + 16, y + 19, C.text);
}

function renderFrame(frame) {
  const buffer = frameBuffer();
  for (let x = 0; x < WIDTH; x += 40) {
    fillRect(buffer, x, 0, 1, HEIGHT, C.surface);
  }
  for (let y = 0; y < HEIGHT; y += 40) {
    fillRect(buffer, 0, y, WIDTH, 1, C.surface);
  }

  drawText(buffer, "LUME WHITEBOARD", 40, 24, C.text, 2);
  drawText(buffer, "MOVE CLOSE / HIGHLIGHT / RELEASE / DOCK", 41, 48, C.muted);

  const firstX = 40;
  const targetX = 370;
  const initialX = 480;
  const terminalY = 88;
  const terminalWidth = 330;
  const terminalHeight = 260;
  let secondX = initialX;
  let moving = false;
  let docked = false;

  if (frame >= 7 && frame <= 24) {
    moving = true;
    const progress = (frame - 7) / 17;
    const eased = 0.5 - Math.cos(progress * Math.PI) / 2;
    secondX = Math.round(initialX + (targetX - initialX) * eased);
  } else if (frame === 25) {
    moving = true;
    secondX = targetX - 7;
  } else if (frame === 26) {
    moving = true;
    secondX = targetX + 3;
  } else if (frame >= 27) {
    secondX = targetX;
    docked = true;
  }

  const showPreview = moving && secondX - targetX < 58;
  if (showPreview) {
    drawDashedPreview(
      buffer,
      targetX,
      terminalY,
      terminalWidth,
      terminalHeight,
      frame % 4 < 2,
    );
  }

  drawTerminal(buffer, {
    x: firstX,
    y: terminalY,
    width: terminalWidth,
    height: terminalHeight,
    agent: "CODEX",
    project: "LUME",
    source: "VS CODE",
    running: true,
    moving: false,
    docked,
    frame,
  });
  drawTerminal(buffer, {
    x: secondX,
    y: terminalY,
    width: terminalWidth,
    height: terminalHeight,
    agent: "CLAUDE",
    project: "ORBIT API",
    source: "CLI",
    running: false,
    moving,
    docked,
    frame,
  });

  if (moving) {
    drawCursor(buffer, secondX + 236, terminalY + 24);
  }
  if (docked) {
    const label = "DOCKED / THE WINDOWS NOW MOVE AS A GROUP";
    const width = textWidth(label) + 24;
    roundedRect(buffer, (WIDTH - width) / 2, 365, width, 22, 10, C.border);
    drawText(buffer, label, (WIDTH - width) / 2 + 12, 373, C.green);
  } else {
    drawText(buffer, "DRAG THE CLAUDE WINDOW TOWARD CODEX", 40, 373, C.muted);
  }
  return buffer;
}

function pushWord(bytes, value) {
  bytes.push(value & 0xff, (value >> 8) & 0xff);
}

function literalLzw(indices) {
  const clearCode = 8;
  const endCode = 9;
  const bytes = [];
  let current = 0;
  let bitCount = 0;
  const writeCode = (code) => {
    current |= code << bitCount;
    bitCount += 4;
    while (bitCount >= 8) {
      bytes.push(current & 0xff);
      current >>= 8;
      bitCount -= 8;
    }
  };

  for (let index = 0; index < indices.length; index += 6) {
    writeCode(clearCode);
    const end = Math.min(indices.length, index + 6);
    for (let cursor = index; cursor < end; cursor += 1) {
      writeCode(indices[cursor]);
    }
  }
  writeCode(endCode);
  if (bitCount > 0) bytes.push(current & 0xff);
  return bytes;
}

function encodeGif(frames) {
  const bytes = [...Buffer.from("GIF89a", "ascii")];
  pushWord(bytes, WIDTH);
  pushWord(bytes, HEIGHT);
  bytes.push(0xf2, C.background, 0);
  for (const [red, green, blue] of palette) bytes.push(red, green, blue);

  bytes.push(
    0x21, 0xff, 0x0b,
    ...Buffer.from("NETSCAPE2.0", "ascii"),
    0x03, 0x01, 0x00, 0x00, 0x00,
  );

  for (const frame of frames) {
    bytes.push(0x21, 0xf9, 0x04, 0x04);
    pushWord(bytes, FRAME_DELAY);
    bytes.push(0x00, 0x00);
    bytes.push(0x2c);
    pushWord(bytes, 0);
    pushWord(bytes, 0);
    pushWord(bytes, WIDTH);
    pushWord(bytes, HEIGHT);
    bytes.push(0x00, 0x03);

    const compressed = literalLzw(frame);
    for (let offset = 0; offset < compressed.length; offset += 255) {
      const block = compressed.slice(offset, offset + 255);
      bytes.push(block.length, ...block);
    }
    bytes.push(0x00);
  }
  bytes.push(0x3b);
  return Buffer.from(bytes);
}

const frames = Array.from({ length: FRAME_COUNT }, (_, frame) => renderFrame(frame));
mkdirSync(dirname(outputPath), { recursive: true });
writeFileSync(outputPath, encodeGif(frames));
console.log(`Generated ${outputPath} (${FRAME_COUNT} frames, ${(FRAME_COUNT * FRAME_DELAY) / 100}s)`);
