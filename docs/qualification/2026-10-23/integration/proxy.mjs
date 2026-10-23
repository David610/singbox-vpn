import http from "node:http";
http.createServer((req, res) => {
  const path = req.url.replace(/^\/rest\/v1/, "") || "/";
  const up = http.request({ host: "127.0.0.1", port: 3000, path, method: req.method, headers: req.headers }, (r) => { res.writeHead(r.statusCode, r.headers); r.pipe(res); });
  up.on("error", () => { res.writeHead(502); res.end(); }); req.pipe(up);
}).listen(3001, "127.0.0.1");
