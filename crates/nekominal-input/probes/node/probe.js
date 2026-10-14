// Raw-stdin probe. Usage: node probe.js <logfile> [setup-bytes-as-hex]
// Writes the setup bytes to stdout (the terminal modes the "application" asks for), prints READY,
// then logs every chunk that arrives on stdin as hex, one line per chunk.
const fs = require('fs');

const logFile = process.argv[2];
const setup = process.argv[3] ? Buffer.from(process.argv[3], 'hex') : null;
const started = Date.now();

function log(line) {
  fs.appendFileSync(logFile, line + '\n');
}

if (process.stdin.isTTY) process.stdin.setRawMode(true);
log(`INFO node=${process.version} isTTY=${process.stdin.isTTY} raw=${process.stdin.isRaw}`);

process.stdin.on('data', (chunk) => {
  log(`DATA t=${Date.now() - started} ${chunk.toString('hex')}`);
});

if (setup) process.stdout.write(setup);
process.stdout.write('READY\n');
setInterval(() => {}, 1000);
