const { JSDOM } = require("jsdom");
let rough = require("roughjs"); rough = rough.default || rough;

const W = 1200, H = 628;
const dom = new JSDOM("<!DOCTYPE html><body></body>");
const doc = dom.window.document;
const NS = "http://www.w3.org/2000/svg";
const svg = doc.createElementNS(NS, "svg");
svg.setAttribute("xmlns", NS); svg.setAttribute("width", W); svg.setAttribute("height", H);
svg.setAttribute("viewBox", `0 0 ${W} ${H}`);
const rc = rough.svg(svg);

const GREEN = "#2ea44f", GREEN_D = "#1a7f37", INK = "#24292f", MUTE = "#57606a";
const FONT = "Marker Felt, Comic Sans MS, sans-serif";
const add = (n) => svg.appendChild(n);
function text(x, y, s, { size = 16, fill = INK, weight = "normal", anchor = "start" } = {}) {
  const t = doc.createElementNS(NS, "text");
  t.setAttribute("x", x); t.setAttribute("y", y); t.setAttribute("font-family", FONT);
  t.setAttribute("font-size", size); t.setAttribute("font-weight", weight);
  t.setAttribute("fill", fill); t.setAttribute("text-anchor", anchor);
  t.textContent = s; add(t);
}
function hex(cx, cy, r) {
  const p = [];
  for (let i = 0; i < 6; i++) { const a = (Math.PI / 180) * (60 * i - 30); p.push([cx + r * Math.cos(a), cy + r * Math.sin(a)]); }
  return p;
}

add(rc.rectangle(0, 0, W, H, { fill: "#fbfdfc", fillStyle: "solid", stroke: "#fbfdfc", roughness: 0 }));

// left: title block
text(72, 116, "MARROW", { size: 22, weight: "bold", fill: GREEN_D });
text(70, 196, "A cure for amnesiac", { size: 52, weight: "bold" });
text(70, 256, "AI agents", { size: 52, weight: "bold" });
text(72, 312, "and a hive mind, so a swarm of them works as one.", { size: 23, fill: INK });
text(72, 346, "one shared memory · always up to date · fully auditable", { size: 19, fill: MUTE });
// chips
function chip(x, y, label) {
  add(rc.rectangle(x, y, 20 + label.length * 11, 38, { fill: "#eaf7ee", fillStyle: "solid", stroke: GREEN_D, strokeWidth: 1.6, roughness: 1.4 }));
  text(x + 12, y + 25, label, { size: 16, fill: GREEN_D });
}
chip(72, 420, "open source");
chip(220, 420, "runs locally");
chip(372, 420, "over MCP");

// right: the hive
const cx = 900, cy = 330, R = 190;
const agents = 6;
for (let i = 0; i < agents; i++) {
  const a = (Math.PI * 2 * i) / agents - Math.PI / 2;
  const ax = cx + R * Math.cos(a), ay = cy + R * Math.sin(a);
  // pheromone trail
  add(rc.line(cx, cy, ax, ay, { stroke: GREEN, strokeWidth: 1.8, roughness: 1.8, bowing: 2 }));
  add(rc.polygon(hex(ax, ay, 40), { fill: "#eaf7ee", fillStyle: "hachure", fillWeight: 1.5, hachureGap: 4, stroke: GREEN_D, strokeWidth: 1.8, roughness: 1.6, seed: 10 + i }));
  text(ax, ay + 5, "agent", { size: 13, fill: MUTE, anchor: "middle" });
}
// central shared brain
add(rc.polygon(hex(cx, cy, 92), { fill: GREEN, fillStyle: "solid", stroke: GREEN_D, strokeWidth: 2.5, roughness: 1.4, seed: 7 }));
text(cx, cy - 4, "shared", { size: 20, weight: "bold", fill: "#ffffff", anchor: "middle" });
text(cx, cy + 22, "brain", { size: 20, weight: "bold", fill: "#ffffff", anchor: "middle" });

process.stdout.write('<?xml version="1.0" encoding="UTF-8"?>\n' + svg.outerHTML + "\n");
