const rows: [number, string][] = [
  [0x01, "Escape Digit1 Digit2 Digit3 Digit4 Digit5 Digit6 Digit7 Digit8 Digit9 Digit0 Minus Equal Backspace"],
  [0x0f, "Tab KeyQ KeyW KeyE KeyR KeyT KeyY KeyU KeyI KeyO KeyP BracketLeft BracketRight Enter ControlLeft"],
  [0x1e, "KeyA KeyS KeyD KeyF KeyG KeyH KeyJ KeyK KeyL Semicolon Quote Backquote ShiftLeft Backslash"],
  [0x2c, "KeyZ KeyX KeyC KeyV KeyB KeyN KeyM Comma Period Slash ShiftRight NumpadMultiply AltLeft Space CapsLock"],
  [0x3b, "F1 F2 F3 F4 F5 F6 F7 F8 F9 F10 NumLock ScrollLock Numpad7 Numpad8 Numpad9 NumpadSubtract Numpad4 Numpad5 Numpad6 NumpadAdd Numpad1 Numpad2 Numpad3 Numpad0 NumpadDecimal"],
  [0x56, "IntlBackslash F11 F12"],
];

const keys = new Map<string, number>([
  ...rows.flatMap(([start, row]) => row.split(" ").map<[string, number]>((code, offset) => [code, start + offset])),
  ["NumpadEnter", 0xe01c], ["ControlRight", 0xe01d], ["NumpadDivide", 0xe035],
  ["PrintScreen", 0xe037], ["AltRight", 0xe038], ["Home", 0xe047], ["ArrowUp", 0xe048],
  ["PageUp", 0xe049], ["ArrowLeft", 0xe04b], ["ArrowRight", 0xe04d], ["End", 0xe04f],
  ["ArrowDown", 0xe050], ["PageDown", 0xe051], ["Insert", 0xe052], ["Delete", 0xe053],
  ["MetaLeft", 0xe05b], ["MetaRight", 0xe05c], ["ContextMenu", 0xe05d],
]);

const punctuation = [
  ["Backquote", "`", "~"], ["Minus", "-", "_"], ["Equal", "=", "+"],
  ["BracketLeft", "[", "{"], ["BracketRight", "]", "}"], ["Backslash", "\\", "|"],
  ["Semicolon", ";", ":"], ["Quote", "'", '"'], ["Comma", ",", "<"],
  ["Period", ".", ">"], ["Slash", "/", "?"], ["Space", " ", " "],
];

const keyAliases = new Map(punctuation.flatMap(([code, plain, shifted]) => [
  [plain, code], [shifted, code],
]));
for (const [digit, shifted] of Array.from(")!@#$%^&*(").entries()) {
  keyAliases.set(String(digit), `Digit${digit}`);
  keyAliases.set(shifted, `Digit${digit}`);
}

const numpad = new Map([
  ["Enter", "NumpadEnter"], [".", "NumpadDecimal"], ["+", "NumpadAdd"], ["-", "NumpadSubtract"],
  ["*", "NumpadMultiply"], ["/", "NumpadDivide"], ["Insert", "Numpad0"], ["End", "Numpad1"],
  ["ArrowDown", "Numpad2"], ["PageDown", "Numpad3"], ["ArrowLeft", "Numpad4"], ["Clear", "Numpad5"],
  ["ArrowRight", "Numpad6"], ["Home", "Numpad7"], ["ArrowUp", "Numpad8"], ["PageUp", "Numpad9"],
  ["Delete", "NumpadDecimal"],
]);

export function scanCode({ code, key, location }: Pick<KeyboardEvent, "code" | "key" | "location">) {
  const physical = keys.get(code);
  if (physical !== undefined) return physical;
  // WebView2 can supply only a logical key for Windows virtual-key input.
  if (location === 3) {
    const mapped = numpad.get(key) ?? (/^[0-9]$/.test(key) ? `Numpad${key}` : undefined);
    if (mapped) return keys.get(mapped);
  }
  if (key === "Control" || key === "Shift" || key === "Alt" || key === "Meta") {
    return keys.get(`${key}${location === 2 ? "Right" : "Left"}`);
  }
  if (key === "AltGraph") return keys.get("AltRight");
  if (/^[a-z]$/i.test(key)) return keys.get(`Key${key.toUpperCase()}`);
  return keys.get(keyAliases.get(key) ?? key);
}
