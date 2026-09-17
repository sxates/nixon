import http from 'node:http';
import { readFile, stat } from 'node:fs/promises';
import { extname, join } from 'node:path';
const ROOT=process.argv[2], PORT=+(process.argv[3]||3211);
const MIME={'.html':'text/html','.js':'text/javascript','.css':'text/css','.json':'application/json','.svg':'image/svg+xml','.png':'image/png','.ico':'image/x-icon','.txt':'text/plain','.woff2':'font/woff2','.woff':'font/woff','.map':'application/json'};
const server = http.createServer(async (req,res)=>{
  let p=decodeURIComponent(req.url.split('?')[0]); let fp=join(ROOT,p);
  let s=await stat(fp).catch(()=>null);
  if(s&&s.isDirectory()){fp=join(fp,'index.html');s=await stat(fp).catch(()=>null);}
  if(!s){const h=join(ROOT,(p==='/'?'/index':p)+'.html');const hs=await stat(h).catch(()=>null);if(hs)fp=h;else{res.writeHead(404);res.end('404');return;}}
  try{const d=await readFile(fp);res.writeHead(200,{'content-type':MIME[extname(fp)]||'application/octet-stream'});res.end(d);}catch(e){res.writeHead(500);res.end(''+e);}
});
server.on('error', (e) => { console.error(e.message); process.exit(1); });
server.listen(PORT, () => console.log('serving ' + ROOT + ' :' + server.address().port));
