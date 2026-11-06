// TCP echo server for the e2e action steps: `bun echo-server.ts <port>`.
// Echoes every byte back; after the peer half-closes, flushes and closes.
import net from 'node:net';

const port = Number(process.argv[2]);
if (!Number.isInteger(port) || port < 1 || port > 65535) {
  console.error('usage: bun echo-server.ts <port>');
  process.exit(2);
}

const server = net.createServer({ allowHalfOpen: true }, (sock) => {
  sock.on('data', (chunk) => sock.write(chunk));
  sock.on('end', () => sock.end());
  sock.on('error', () => sock.destroy());
});
server.listen(port, '127.0.0.1', () => console.log(`echo listening on 127.0.0.1:${port}`));
