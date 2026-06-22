const { JSDOM } = require("jsdom");
let rough = require("roughjs");
rough = rough.default || rough;

const W = 780, H = 430;
const dom = new JSDOM("<!DOCTYPE html><body></body>");
const doc = dom.window.document;
const NS = "http://www.w3.org/2000/svg";
const svg = doc.createElementNS(NS, "svg");
svg.setAttribute("xmlns", NS);
svg.setAttribute("width", W); svg.setAttribute("height", H);
svg.setAttribute("viewBox", `0 0 ${W} ${H}`);
const rc = rough.svg(svg);

const GREEN = "#2ea44f", GREEN_D = "#1a7f37";
const GREY = "#aeb4ba", GREY_D = "#6b7075";
const INK = "#24292f", MUTE = "#57606a";
const FONT = "Marker Felt, Comic Sans MS, sans-serif";

function text(x, y, s, { size = 14, fill = INK, weight = "normal", anchor = "start" } = {}) {
  const t = doc.createElementNS(NS, "text");
  t.setAttribute("x", x); t.setAttribute("y", y);
  t.setAttribute("font-family", FONT); t.setAttribute("font-size", size);
  t.setAttribute("font-weight", weight); t.setAttribute("fill", fill);
  t.setAttribute("text-anchor", anchor);
  t.textContent = s; svg.appendChild(t);
}
const add = (n) => svg.appendChild(n);

add(rc.rectangle(6, 6, W - 12, H - 12, { fill: "#fff", fillStyle: "solid", stroke: "#d0d7de", strokeWidth: 1.5, roughness: 1 }));
text(28, 42, "Benchmarks — measured, not asserted", { size: 24, weight: "bold" });

// ---------- Efficiency: comparison bars ----------
text(28, 70, "Efficiency · same task, with vs without Marrow · 5-run A/B", { size: 13, fill: MUTE });
const BX = 120, BW = 470;                       // bar origin + full scale width
const tScale = 180000, sScale = 30;

function bar(y, frac, color, colorD, seed) {
  add(rc.rectangle(BX, y, Math.max(20, BW * frac), 24, { fill: color, fillStyle: "hachure", fillWeight: 2.4, hachureGap: 4.5, stroke: colorD, strokeWidth: 1.8, roughness: 1.4, bowing: 1, seed }));
}
// Tokens
text(28, 112, "Tokens", { size: 15, weight: "bold" });
// variance whisker on cold (98k–170k) — the un-fakeable proof that warm is consistent
const xk = (k) => BX + (k / tScale) * BW;
add(rc.line(xk(98000), 96, xk(170000), 96, { stroke: GREY_D, strokeWidth: 1.6, roughness: 1 }));
add(rc.line(xk(98000), 92, xk(98000), 100, { stroke: GREY_D, strokeWidth: 1.6 }));
add(rc.line(xk(170000), 92, xk(170000), 100, { stroke: GREY_D, strokeWidth: 1.6 }));
text(xk(170000) + 8, 100, "swings 98–170k", { size: 11, fill: MUTE });
bar(104, 134000 / tScale, GREY, GREY_D, 11);
text(BX + BW * (134000 / tScale) + 10, 122, "~134k", { size: 14, weight: "bold", fill: GREY_D });
bar(136, 38000 / tScale, GREEN, GREEN_D, 12);
text(BX + BW * (38000 / tScale) + 10, 154, "~38k · flat every run", { size: 14, weight: "bold", fill: GREEN_D });
text(W - 36, 138, "−72%", { size: 26, weight: "bold", fill: GREEN_D, anchor: "end" });

// Time
text(28, 196, "Time", { size: 15, weight: "bold" });
const xs = (s) => (s / sScale) * BW;
bar(184, 26 / sScale, GREY, GREY_D, 21);
text(BX + xs(26) + 10, 202, "~26s", { size: 14, weight: "bold", fill: GREY_D });
bar(216, 11 / sScale, GREEN, GREEN_D, 22);
text(BX + xs(11) + 10, 234, "~11s", { size: 14, weight: "bold", fill: GREEN_D });
text(W - 36, 214, "−57%", { size: 26, weight: "bold", fill: GREEN_D, anchor: "end" });

// legend
add(rc.rectangle(120, 250, 16, 12, { fill: GREY, fillStyle: "hachure", stroke: GREY_D, strokeWidth: 1.4, roughness: 1 }));
text(142, 261, "without Marrow", { size: 12, fill: MUTE });
add(rc.rectangle(270, 250, 16, 12, { fill: GREEN, fillStyle: "hachure", stroke: GREEN_D, strokeWidth: 1.4, roughness: 1 }));
text(292, 261, "with Marrow", { size: 12, fill: MUTE });

// ---------- Engine: gauge bars ----------
text(28, 300, "Engine accuracy · reproducible offline (cargo run -p marrow-bench)", { size: 13, fill: MUTE });
const GX = 300, GW = 380;
function gauge(y, label, pct, val, seed) {
  text(28, y + 16, label, { size: 13 });
  add(rc.rectangle(GX, y, GW, 20, { fill: "#f0f3f6", fillStyle: "solid", stroke: "#d0d7de", strokeWidth: 1.2, roughness: 0.8 }));
  add(rc.rectangle(GX, y, GW * (pct / 100), 20, { fill: GREEN, fillStyle: "solid", stroke: GREEN_D, strokeWidth: 1.6, roughness: 1.2, seed }));
  text(GX + GW + 12, y + 16, val, { size: 14, weight: "bold", fill: GREEN_D });
}
gauge(314, "Stale-knowledge recall  (~1% false positive)", 98, "98%", 31);
gauge(348, "Consolidation precision  (0 false merges)", 100, "100%", 32);
gauge(382, "Retrieval payload, under token budget", 82, "−82%", 33);

process.stdout.write('<?xml version="1.0" encoding="UTF-8"?>\n' + svg.outerHTML + "\n");
