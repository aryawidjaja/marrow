const { JSDOM } = require("jsdom");
let rough = require("roughjs"); rough = rough.default || rough;
const W = 1040, H = 440;
const dom = new JSDOM("<!DOCTYPE html><body></body>"); const doc = dom.window.document;
const NS = "http://www.w3.org/2000/svg";
const svg = doc.createElementNS(NS, "svg");
svg.setAttribute("xmlns", NS); svg.setAttribute("width", W); svg.setAttribute("height", H); svg.setAttribute("viewBox", `0 0 ${W} ${H}`);
const rc = rough.svg(svg);
const GREEN = "#2ea44f", GREEN_D = "#1a7f37", INK = "#24292f", MUTE = "#57606a";
const FONT = "Marker Felt, Comic Sans MS, sans-serif";
const add = (n) => svg.appendChild(n);
function text(x, y, s, o = {}) { const { size = 16, fill = INK, weight = "normal", anchor = "start" } = o; const t = doc.createElementNS(NS, "text"); t.setAttribute("x", x); t.setAttribute("y", y); t.setAttribute("font-family", FONT); t.setAttribute("font-size", size); t.setAttribute("font-weight", weight); t.setAttribute("fill", fill); t.setAttribute("text-anchor", anchor); t.textContent = s; add(t); }

add(rc.rectangle(6, 6, W - 12, H - 12, { fill: "#fff", fillStyle: "solid", stroke: "#d0d7de", strokeWidth: 1.5, roughness: 1 }));
text(36, 50, "Memory like a database, not a file you keep nudging", { size: 24, weight: "bold" });

// database cylinder (shared memory)
const cx = 720, top = 120, w = 200, h = 180, eh = 46;
function ellipse(yy, fill) { return rc.ellipse(cx, yy, w, eh, { fill, fillStyle: "solid", stroke: GREEN_D, strokeWidth: 2, roughness: 1.3 }); }
add(rc.rectangle(cx - w / 2, top, w, h, { fill: "#eaf7ee", fillStyle: "solid", stroke: "#eaf7ee", roughness: 0 }));
add(rc.line(cx - w / 2, top, cx - w / 2, top + h, { stroke: GREEN_D, strokeWidth: 2, roughness: 1.2 }));
add(rc.line(cx + w / 2, top, cx + w / 2, top + h, { stroke: GREEN_D, strokeWidth: 2, roughness: 1.2 }));
add(ellipse(top + h, "#eaf7ee"));
add(ellipse(top, GREEN));
text(cx, top + h / 2, "Marrow", { size: 22, weight: "bold", fill: GREEN_D, anchor: "middle" });
text(cx, top + h / 2 + 26, "one source of truth", { size: 14, fill: MUTE, anchor: "middle" });
text(cx, top + h / 2 + 48, "always up to date", { size: 14, fill: MUTE, anchor: "middle" });

// agents read + write
const agents = ["Claude Code", "Cursor", "Codex"];
for (let i = 0; i < 3; i++) {
  const ax = 90, ay = 130 + i * 90;
  add(rc.rectangle(ax, ay, 180, 56, { fill: "#fbfbfc", fillStyle: "solid", stroke: "#6b7075", strokeWidth: 1.8, roughness: 1.4, seed: 4 + i }));
  text(ax + 90, ay + 34, agents[i], { size: 16, anchor: "middle" });
  add(rc.line(ax + 185, ay + 28, cx - w / 2 - 6, top + h / 2, { stroke: GREEN, strokeWidth: 1.8, roughness: 1.6, bowing: 2 }));
}
text(280, 408, "read + write the same memory, so every agent and every session stays in sync.", { size: 14, fill: MUTE });
process.stdout.write('<?xml version="1.0" encoding="UTF-8"?>\n' + svg.outerHTML + "\n");
