// A stand-in for what a full-screen program (Claude Code, vim, tmux) sees of the pointer and the clipboard: it reads
// stdin raw, so ConPTY passes every sequence through.
//
//   node input-probe.js <modes> <osc52> <milliseconds>
//
// <modes>   DECSET numbers to turn on, for example 1000,1002,1006 (mouse press and drag, SGR encoding); "" for none.
// <osc52>   "osc52" asks the terminal for the clipboard (OSC 52 read) first; "" does not.
// It prints "got:[...]" once the time is up, every byte it received with ESC as <ESC> and BEL as <BEL>, and turns the
// modes off again.
const ESC = String.fromCharCode(27);
const BEL = String.fromCharCode(7);
const [modesArg = "", osc52 = "", millis = "5000"] = process.argv.slice(2);
const modes = modesArg.split(",").filter(Boolean);

process.stdin.setRawMode(true);
process.stdin.resume();
for (const mode of modes) process.stdout.write(`${ESC}[?${mode}h`);
if (osc52 === "osc52") process.stdout.write(`${ESC}]52;c;?${BEL}`);

let got = "";
process.stdin.on("data", (chunk) => {
  got += chunk.toString().split(ESC).join("<ESC>").split(BEL).join("<BEL>");
});

setTimeout(() => {
  for (const mode of modes) process.stdout.write(`${ESC}[?${mode}l`);
  process.stdout.write(`\ngot:[${got}]\n`);
  process.exit(0);
}, Number(millis));
