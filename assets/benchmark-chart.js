const { JSDOM } = require("jsdom");
let rough = require("roughjs");
rough = rough.default || rough;

const W = 760, H = 332;
const dom = new JSDOM("<!DOCTYPE html><body></body>");
const doc = dom.window.document;
const NS = "http://www.w3.org/2000/svg";
const svg = doc.createElementNS(NS, "svg");
svg.setAttribute("xmlns", NS);
svg.setAttribute("width", W);
svg.setAttribute("height", H);
svg.setAttribute("viewBox", `0 0 ${W} ${H}`);
const rc = rough.svg(svg);

const GREEN = "#1a7f37", INK = "#24292f", MUTE = "#57606a";
const FONT = "Marker Felt, Comic Sans MS, sans-serif";

function text(x, y, s, { size = 14, fill = INK, weight = "normal", anchor = "start" } = {}) {
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

svg.appendChild(rc.rectangle(6, 6, W - 12, H - 12, { fill: "#ffffff", fillStyle: "solid", stroke: "#d0d7de", strokeWidth: 1.5, roughness: 1.1 }));
text(28, 44, "Benchmarks — measured, not asserted", { size: 24, weight: "bold" });

const cols = [28, 274, 520];
const tw = 212, th = 96;

function tile(cx, cy, big, label, sub, seed) {
  svg.appendChild(rc.rectangle(cx, cy, tw, th, { fill: "#f6fef9", fillStyle: "solid", stroke: GREEN, strokeWidth: 2, roughness: 1.5, bowing: 1.2, seed }));
  const mid = cx + tw / 2;
  text(mid, cy + 46, big, { size: 38, weight: "bold", fill: GREEN, anchor: "middle" });
  text(mid, cy + 70, label, { size: 14, fill: INK, anchor: "middle" });
  if (sub) text(mid, cy + 88, sub, { size: 11.5, fill: MUTE, anchor: "middle" });
}

// Row 1 — efficiency
text(28, 80, "Efficiency · same task with vs without Marrow · 5-run A/B", { size: 13, fill: MUTE });
tile(cols[0], 90, "72%", "fewer tokens", "134k → 38k", 11);
tile(cols[1], 90, "57%", "faster", "26s → 11s", 12);
tile(cols[2], 90, "25%", "cheaper", "per query", 13);

// Row 2 — engine
text(28, 222, "Engine · reproducible offline with  cargo run -p marrow-bench", { size: 13, fill: MUTE });
tile(cols[0], 232, "98%", "stale-knowledge recall", "~1% false positive", 21);
tile(cols[1], 232, "100%", "consolidation precision", "0 false merges", 22);
tile(cols[2], 232, "82%", "smaller retrieval payload", "under token budget", 23);

process.stdout.write('<?xml version="1.0" encoding="UTF-8"?>\n' + svg.outerHTML + "\n");
