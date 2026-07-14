import http from "node:http";
import fs from "node:fs";
import path from "node:path";

const [rootArgument, portArgument] = process.argv.slice(2);
if (!rootArgument || !portArgument) {
  throw new Error("usage: node tests/static_server.mjs <root> <port>");
}
const root = path.resolve(rootArgument);
const port = Number(portArgument);
const contentTypes = {
  ".css": "text/css; charset=utf-8",
  ".html": "text/html; charset=utf-8",
  ".json": "application/json; charset=utf-8",
};

http.createServer((request, response) => {
  const pathname = decodeURIComponent(new URL(request.url, "http://localhost").pathname);
  const relative = pathname === "/" ? "index.html" : pathname.slice(1);
  const file = path.resolve(root, relative);
  if (!file.startsWith(`${root}${path.sep}`) && file !== root) {
    response.writeHead(403).end("forbidden");
    return;
  }
  fs.readFile(file, (error, bytes) => {
    if (error) {
      response.writeHead(404).end("not found");
      return;
    }
    response.writeHead(200, {
      "Content-Type": contentTypes[path.extname(file)] ?? "application/octet-stream",
    });
    response.end(bytes);
  });
}).listen(port, "127.0.0.1", () => {
  process.stdout.write(`serving ${root} at http://127.0.0.1:${port}/\n`);
});
