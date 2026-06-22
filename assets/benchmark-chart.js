const { JSDOM } = require("jsdom");
let rough = require("roughjs");
rough = rough.default || rough;

const W = 880, H = 470;
const dom = new JSDOM("<!DOCTYPE html><body></body>");
const doc = dom.window.document;
const NS = "http://www.w3.org/2000/svg";
const svg = doc.createElementNS(NS, "svg");
svg.setAttribute("xmlns", NS);
svg.setAttribute("width", W);
svg.setAttribute("height", H);
svg.setAttribute("viewBox", `0 0 ${W} ${H}`);
const rc = rough.svg(svg);

const GREEN = "#2ea44f", GREEN_D = "#1a7f37";
const GREY = "#9aa0a6", GREY_D = "#6b7075";
const INK = "#24292f", MUTE = "#57606a";
const FONT = "Marker Felt, Comic Sans MS, sans-serif";

function text(x, y, s, { size = 16, fill = INK, weight = "normal", anchor = "start" } = {}) {
  const t = doc.createElementNS(NS, "text");
  t.setAttribute("x", x); t.setAttribute("y", y);
  t.setAttribute("font-family", FONT);
  t.setAttribute("font-size", size);
  t.setAttribute("font-weight", weight);
  t.setAttribute("fill", fill);
  t.setAttribute("text-anchor", anchor);
  t.textContent = s;
  svg.appendChild(t);
}
function add(node) { svg.appendChild(node); }

// card background
add(rc.rectangle(8, 8, W - 16, H - 16, { fill: "#ffffff", fillStyle: "solid", stroke: "#d0d7de", strokeWidth: 1.5, roughness: 1.2 }));

// title + subtitle
text(40, 58, "What it costs to understand this repo", { size: 30, weight: "bold" });
text(40, 88, "Claude Code, same question, with vs without Marrow  ·  5-run average  ·  less is better", { size: 15, fill: MUTE });

const BX = 250;            // bar start x
const MAXW = 470;          // full-scale bar width
const valX = BX + MAXW + 18;

function row(y, label, frac, color, colorD, valStr) {
  text(40, y + 24, label, { size: 16, fill: INK });
  const w = Math.max(26, MAXW * frac);
  add(rc.rectangle(BX, y, w, 34, { fill: color, fillStyle: "hachure", fillWeight: 2.5, hachureGap: 5, stroke: colorD, strokeWidth: 2, roughness: 1.5, bowing: 1.2 }));
  text(BX + w + 14, y + 24, valStr, { size: 18, weight: "bold", fill: colorD });
}

// ---- Tokens ----
text(40, 140, "Tokens", { size: 20, weight: "bold", fill: MUTE });
row(152, "Without Marrow", 1.0, GREY, GREY_D, "~134k");
row(198, "With Marrow", 38 / 134, GREEN, GREEN_D, "~38k");
// savings badge
add(rc.rectangle(valX + 70, 150, 150, 50, { fill: "#dafbe1", fillStyle: "solid", stroke: GREEN_D, strokeWidth: 2, roughness: 1.6 }));
text(valX + 145, 183, "72% fewer", { size: 22, weight: "bold", fill: GREEN_D, anchor: "middle" });

// ---- Time ----
text(40, 290, "Wall-clock time", { size: 20, weight: "bold", fill: MUTE });
row(302, "Without Marrow", 1.0, GREY, GREY_D, "~26s");
row(348, "With Marrow", 11 / 26, GREEN, GREEN_D, "~11s");
add(rc.rectangle(valX + 70, 300, 150, 50, { fill: "#dafbe1", fillStyle: "solid", stroke: GREEN_D, strokeWidth: 2, roughness: 1.6 }));
text(valX + 145, 333, "57% faster", { size: 22, weight: "bold", fill: GREEN_D, anchor: "middle" });

// footer
text(40, 442, "Warm stays flat (~38k every run); cold re-reads the repo, so the gap widens on bigger codebases.", { size: 14, fill: MUTE });

process.stdout.write('<?xml version="1.0" encoding="UTF-8"?>\n' + svg.outerHTML + "\n");
