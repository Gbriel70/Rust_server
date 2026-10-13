#!/usr/bin/env python3
"""Byte-exact ranges, large-file RSS, cancellation, and actual Chrome video seek."""
import hashlib,http.client,json,pathlib,socket,subprocess,tempfile,threading,time,urllib.request
import websocket
ROOT=pathlib.Path(__file__).resolve().parents[1]
OUT=ROOT/'validation/phase11';OUT.mkdir(parents=True,exist_ok=True)
class Server:
 def __init__(self,root):
  self.log=open(OUT/'server.log','w');self.p=subprocess.Popen([str(ROOT/'target/release/rust_serv'),'--root',str(root),'--mode','tokio','--addr','127.0.0.1:0','--idle-timeout','30'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=self.log,text=True)
  self.addr=self.p.stdout.readline().split()[2];self.port=int(self.addr.rsplit(':',1)[1])
 def close(self):
  self.p.stdin.write('\n');self.p.stdin.flush();self.p.wait(timeout=15);self.log.close()
 def get(self,path,headers=None):
  c=http.client.HTTPConnection('127.0.0.1',self.port,timeout=15);c.request('GET',path,headers=headers or {});r=c.getresponse();body=r.read();result=(r.status,dict(r.getheaders()),body);c.close();return result
 def rss(self):return int(next(x for x in pathlib.Path(f'/proc/{self.p.pid}/status').read_text().splitlines() if x.startswith('VmRSS:')).split()[1])
class CDP:
 def __init__(self,url):self.ws=websocket.create_connection(url,timeout=10);self.n=0;self.requests=[];self.responses=[]
 def call(self,method,params=None):
  self.n+=1;ident=self.n;self.ws.send(json.dumps({'id':ident,'method':method,'params':params or {}}))
  while True:
   msg=json.loads(self.ws.recv())
   if msg.get('method')=='Network.requestWillBeSent':self.requests.append(msg['params']['request'])
   if msg.get('method')=='Network.responseReceived':self.responses.append(msg['params']['response'])
   if msg.get('id')==ident:
    if 'error' in msg:raise RuntimeError(msg['error'])
    return msg.get('result',{})
 def evaluate(self,script):return self.call('Runtime.evaluate',{'expression':script,'returnByValue':True})['result'].get('value')
def browser(server):
 with tempfile.TemporaryDirectory(prefix='rust-range-chrome-') as profile:
  log=open(OUT/'chrome.log','w');chrome=subprocess.Popen(['google-chrome','--headless=new','--no-sandbox','--disable-gpu','--disable-background-networking','--disable-component-update','--autoplay-policy=no-user-gesture-required','--remote-allow-origins=*','--remote-debugging-port=0',f'--user-data-dir={profile}','about:blank'],stdout=log,stderr=log)
  cdp=None
  try:
   active=pathlib.Path(profile)/'DevToolsActivePort'
   for _ in range(100):
    if active.exists():break
    if chrome.poll() is not None:raise RuntimeError('Chrome exited; see chrome.log')
    time.sleep(.1)
   port=int(active.read_text().splitlines()[0]);tabs=json.load(urllib.request.urlopen(f'http://127.0.0.1:{port}/json'));tab=next(tab for tab in tabs if tab['type']=='page');cdp=CDP(tab['webSocketDebuggerUrl'])
   for method in ['Page.enable','Runtime.enable','Network.enable']:cdp.call(method)
   cdp.call('Network.setCacheDisabled',{'cacheDisabled':True})
   cdp.call('Network.emulateNetworkConditions',{'offline':False,'latency':20,'downloadThroughput':80*1024,'uploadThroughput':-1})
   cdp.call('Page.navigate',{'url':f'http://{server.addr}/'})
   deadline=time.monotonic()+40
   while not cdp.evaluate('Boolean(document.querySelector("video") && document.querySelector("video").readyState >= 1)'):
    assert time.monotonic()<deadline,'metadata timeout';time.sleep(.2)
   duration=cdp.evaluate('document.querySelector("video").duration');target=duration*.65
   before_seek=len(cdp.requests)
   cdp.evaluate(f'document.querySelector("video").currentTime={target}')
   deadline=time.monotonic()+50
   while not cdp.evaluate(f'document.querySelector("video").currentTime > {target-1} && !document.querySelector("video").seeking && document.querySelector("video").readyState >= 2'):
    assert time.monotonic()<deadline,'seek timeout';time.sleep(.2)
   cdp.evaluate('document.querySelector("video").play().catch(()=>{})');time.sleep(1);position=cdp.evaluate('document.querySelector("video").currentTime')
   responses=[{'status':r['status'],'headers':r['headers']} for r in cdp.responses if r['url'].endswith('Download.mp4')]
   requests=[r['headers'] for r in cdp.requests if r['url'].endswith('Download.mp4')]
   assert sum(r['status']==206 for r in responses)>=2,(requests,responses)
   assert any(r['url'].endswith('Download.mp4') and r['headers'].get('Range','bytes=0-') != 'bytes=0-' for r in cdp.requests[before_seek:]),requests
   result={'duration':duration,'target':target,'position_after_seek':position,'requests':requests,'responses':responses}
   (OUT/'browser-seek.json').write_text(json.dumps(result,indent=2)+'\n');return result
  except Exception:
   if cdp:
    diagnostic={'video':cdp.evaluate('JSON.stringify({url:location.href,html:document.body.innerText,video:document.querySelector("video") ? {error:document.querySelector("video").error?.message,ready:document.querySelector("video").readyState,network:document.querySelector("video").networkState,src:document.querySelector("video").currentSrc} : null})'),'requests':cdp.requests,'responses':cdp.responses}
    (OUT/'browser-failure.json').write_text(json.dumps(diagnostic,indent=2)+'\n')
   raise
  finally:
   if cdp:cdp.ws.close()
   chrome.terminate()
   try:chrome.wait(timeout=10)
   except subprocess.TimeoutExpired:chrome.kill();chrome.wait()
   log.close()
def main():
 with tempfile.TemporaryDirectory(prefix='rust-range-data-') as temp:
  root=pathlib.Path(temp);data=bytes(range(256))*4096;(root/'data.bin').write_bytes(data)
  with (root/'big.bin').open('wb') as f:f.truncate(2*1024**3)
  server=Server(root)
  try:
   checks=[]
   for spec,start,count in [('0-99',0,100),('100-',100,len(data)-100),('-50',len(data)-50,50)]:
    got=root/f'curl-{start}.bin';expected=root/f'dd-{start}.bin'
    subprocess.run(['curl','-fsS','-r',spec,'-o',str(got),f'http://{server.addr}/data.bin'],check=True)
    subprocess.run(['dd',f'if={root}/data.bin',f'of={expected}',f'skip={start}',f'count={count}','iflag=skip_bytes,count_bytes','status=none'],check=True)
    subprocess.run(['cmp',str(got),str(expected)],check=True);checks.append({'range':spec,'bytes':count,'cmp':'identical'})
   status,headers,_=server.get('/data.bin',{'Range':'bytes=99999999-'});assert status==416 and headers['Content-Range']==f'bytes */{len(data)}'
   samples=[server.rss()];stop=threading.Event()
   def monitor():
    while not stop.wait(.01):samples.append(server.rss())
   thread=threading.Thread(target=monitor);thread.start()
   try:
    c=http.client.HTTPConnection('127.0.0.1',server.port,timeout=15);c.request('GET','/big.bin',headers={'Range':f'bytes={1024**3}-'});r=c.getresponse();assert r.status==206;digest=hashlib.sha256();count=0
    while block:=r.read(1024*1024):count+=len(block);digest.update(block)
    c.close();assert count==1024**3
    expected=hashlib.sha256();zero=bytes(1024*1024)
    for _ in range(1024):expected.update(zero)
    assert expected.digest()==digest.digest()
   finally:stop.set();thread.join()
   slow=socket.create_connection(('127.0.0.1',server.port));slow.sendall(b'GET /big.bin HTTP/1.1\r\nHost: x\r\nRange: bytes=1073741824-\r\n\r\n');slow.recv(1024);slow_rss=[]
   for _ in range(30):slow_rss.append(server.rss());time.sleep(.1)
   assert server.get('/health')[0]==200;slow.close();time.sleep(.2)
   result={'curl_dd_cmp':checks,'large_range':{'bytes':count,'offset':1024**3,'rss_min_kib':min(samples),'rss_max_kib':max(samples),'samples':len(samples)},'slow_reader':{'rss_min_kib':min(slow_rss),'rss_max_kib':max(slow_rss)}}
   (OUT/'experiment-output.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result,indent=2),flush=True)
  finally:server.close()
  video=Server(ROOT/'public')
  try:print(json.dumps({'browser':browser(video)},indent=2),flush=True)
  finally:video.close()
if __name__=='__main__':main()
