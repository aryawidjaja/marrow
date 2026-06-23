const { JSDOM } = require("jsdom");
let rough = require("roughjs"); rough = rough.default || rough;
const W = 1040, H = 420;
const dom = new JSDOM("<!DOCTYPE html><body></body>"); const doc = dom.window.document;
const NS = "http://www.w3.org/2000/svg";
const svg = doc.createElementNS(NS, "svg");
svg.setAttribute("xmlns", NS); svg.setAttribute("width", W); svg.setAttribute("height", H); svg.setAttribute("viewBox", `0 0 ${W} ${H}`);
const rc = rough.svg(svg);
const GREY_D = "#6b7075", INK = "#24292f", MUTE = "#57606a", RED = "#b35900";
const FONT = "Marker Felt, Comic Sans MS, sans-serif";
const add = (n) => svg.appendChild(n);
function text(x, y, s, o = {}) { const { size = 16, fill = INK, weight = "normal", anchor = "start" } = o; const t = doc.createElementNS(NS, "text"); t.setAttribute("x", x); t.setAttribute("y", y); t.setAttribute("font-family", FONT); t.setAttribute("font-size", size); t.setAttribute("font-weight", weight); t.setAttribute("fill", fill); t.setAttribute("text-anchor", anchor); t.textContent = s; add(t); }

add(rc.rectangle(6, 6, W - 12, H - 12, { fill: "#fff", fillStyle: "solid", stroke: "#d0d7de", strokeWidth: 1.5, roughness: 1 }));
text(36, 50, "Every new session starts from zero", { size: 26, weight: "bold" });

const cards = [["Session 1", "9:02 am"], ["Session 2", "11:18 am"], ["Session 3", "2:40 pm"]];
for (let i = 0; i < 3; i++) {
  const x = 40 + i * 330, y = 90;
  add(rc.rectangle(x, y, 290, 210, { fill: "#fbfbfc", fillStyle: "solid", stroke: GREY_D, strokeWidth: 1.8, roughness: 1.4, seed: 3 + i }));
  text(x + 20, y + 36, cards[i][0], { size: 18, weight: "bold" });
  text(x + 270, y + 36, cards[i][1], { size: 13, fill: MUTE, anchor: "end" });
  // speech bubble
  add(rc.rectangle(x + 20, y + 56, 250, 80, { fill: "#fff", fillStyle: "solid", stroke: "#aeb4ba", strokeWidth: 1.5, roughness: 1.6, seed: 30 + i }));
  text(x + 34, y + 88, "\"wait, what is this", { size: 15, fill: INK });
  text(x + 34, y + 112, "project again?\"", { size: 15, fill: INK });
  text(x + 20, y + 168, "you paste the context.", { size: 14, fill: MUTE });
  text(x + 20, y + 190, "again.", { size: 16, weight: "bold", fill: RED });
}
text(40, 350, "CLAUDE.md and HANDOFF.md help, but they drift out of date, and you keep asking the agent to update them.", { size: 15, fill: MUTE });
process.stdout.write('<?xml version="1.0" encoding="UTF-8"?>\n' + svg.outerHTML + "\n");
