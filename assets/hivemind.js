const { JSDOM } = require("jsdom");
let rough = require("roughjs"); rough = rough.default || rough;

const W = 1040, H = 470;
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
  t.setAttribute("fill", fill); t.setAttribute("text-anchor", anchor); t.textContent = s; add(t);
}
function hex(cx, cy, r) { const p = []; for (let i = 0; i < 6; i++) { const a = (Math.PI / 180) * (60 * i - 30); p.push([cx + r * Math.cos(a), cy + r * Math.sin(a)]); } return p; }

add(rc.rectangle(6, 6, W - 12, H - 12, { fill: "#fff", fillStyle: "solid", stroke: "#d0d7de", strokeWidth: 1.5, roughness: 1 }));
text(36, 50, "How the hive mind works", { size: 24, weight: "bold" });

const bx = 800, by = 255;
const rows = [
  { y: 130, big: "Joins warm", sub: "already knows what other sessions did" },
  { y: 255, big: "Claims its work", sub: "so two sessions never touch the same file" },
  { y: 380, big: "Senses the swarm", sub: "a live pheromone trail, every turn" },
];
for (let i = 0; i < rows.length; i++) {
  const r = rows[i], ax = 360, ay = r.y;
  add(rc.line(ax + 30, ay, bx - 70, by, { stroke: GREEN, strokeWidth: 1.8, roughness: 1.8, bowing: 2.5 }));
  add(rc.polygon(hex(ax, ay, 40), { fill: "#eaf7ee", fillStyle: "hachure", fillWeight: 1.5, hachureGap: 4, stroke: GREEN_D, strokeWidth: 1.8, roughness: 1.5, seed: 5 + i }));
  text(ax, ay + 5, "agent", { size: 13, fill: MUTE, anchor: "middle" });
  text(300, ay - 6, r.big, { size: 18, weight: "bold", anchor: "end" });
  text(300, ay + 16, r.sub, { size: 13.5, fill: MUTE, anchor: "end" });
}
add(rc.polygon(hex(bx, by, 92), { fill: GREEN, fillStyle: "solid", stroke: GREEN_D, strokeWidth: 2.5, roughness: 1.4, seed: 7 }));
text(bx, by - 4, "shared", { size: 20, weight: "bold", fill: "#fff", anchor: "middle" });
text(bx, by + 22, "brain", { size: 20, weight: "bold", fill: "#fff", anchor: "middle" });
text(bx, by + 120, "plain files · tamper-evident log · runs locally", { size: 14, fill: MUTE, anchor: "middle" });

process.stdout.write('<?xml version="1.0" encoding="UTF-8"?>\n' + svg.outerHTML + "\n");
