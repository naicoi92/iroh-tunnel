// Wait until a TCP port accepts connections:
// `bun wait-port.ts <host:port> [timeoutSeconds=15]`.
// Retries every 200 ms; exits 0 once connected, 1 on timeout, 2 on bad usage.
import net from 'node:net';

const [addr, timeoutArg] = process.argv.slice(2);
const idx = addr?.lastIndexOf(':') ?? -1;
const timeoutS = timeoutArg === undefined ? 15 : Number(timeoutArg);
if (!addr || idx < 1 || !Number.isFinite(timeoutS) || timeoutS <= 0) {
  console.error('usage: bun wait-port.ts <host:port> [timeoutSeconds]');
  process.exit(2);
}
const host = addr.slice(0, idx);
const port = Number(addr.slice(idx + 1));
const deadline = Date.now() + timeoutS * 1000;

function attempt(): void {
  const sock = net.connect({ host, port });
  sock.once('connect', () => {
    sock.destroy();
    console.log(`wait-port ${addr}: listening`);
    process.exit(0);
  });
  sock.once('error', () => {
    sock.destroy();
    if (Date.now() >= deadline) {
      console.error(`wait-port ${addr}: not listening after ${timeoutS}s`);
      process.exit(1);
    }
    setTimeout(attempt, 200);
  });
}
attempt();
