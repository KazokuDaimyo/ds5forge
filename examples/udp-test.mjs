// Sends a few UDP commands to DS5Forge.
// Usage: node examples/udp-test.mjs [controller]   (no argument: all controllers)
// `controller` is a profile name, an identifier (MAC address) or an index.
import dgram from "node:dgram";

const PORT = 6970;
const arg = process.argv[2]; // name, identifier or index; missing = all
const controller = arg !== undefined && /^\d+$/.test(arg) ? Number(arg) : arg;
const socket = dgram.createSocket("udp4");
const send = (message) =>
  new Promise((resolve) => socket.send(JSON.stringify(controller === undefined ? message : { controller, ...message }), PORT, "127.0.0.1", resolve));
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

console.log("Red light bar + right trigger in weapon mode");
await send({
  lightbar: [255, 0, 0],
  triggers: { right: { kind: "weapon", start: 3, end: 6, strength: 8 } },
});
await wait(3000);

console.log("Green light bar + resistance on L2");
await send({
  lightbar: [0, 255, 0],
  triggers: {
    left: { kind: "feedback", start: 2, strength: 7 },
    right: { kind: "off" },
  },
});
await wait(3000);

console.log("Back to the saved profile");
await send({ reset: true });
socket.close();
