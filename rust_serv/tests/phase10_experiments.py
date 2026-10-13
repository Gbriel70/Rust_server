#!/usr/bin/env python3
"""Real TCP experiments; bounded client buffers, sparse 2 GiB fixture, cleanup."""
import hashlib
import http.client
import json
import pathlib
import signal
import subprocess
import tempfile
import threading
import time

PROJECT = pathlib.Path(__file__).resolve().parents[1]
OUT = PROJECT / 'validation/phase10'
OUT.mkdir(parents=True, exist_ok=True)
BINARY = PROJECT / 'target/release/rust_serv'


def rss(pid):
    text = pathlib.Path(f'/proc/{pid}/status').read_text()
    return int(next(line for line in text.splitlines() if line.startswith('VmRSS:')).split()[1])


class Server:
    def __init__(self, root, trace=None):
        self.log = open(OUT / ('server.log' if trace is None else 'trace-server.log'), 'w')
        cmd = [str(BINARY), '--mode', 'tokio', '--addr', '127.0.0.1:0', '--root', str(root), '--idle-timeout', '30']
        if trace:
            cmd = ['strace', '-f', '-c', '-o', str(OUT / 'strace-summary.txt')] + cmd
        self.process = subprocess.Popen(cmd, cwd=PROJECT, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.log, text=True)
        line = self.process.stdout.readline()
        if 'listening' not in line.lower():
            self.close()
            raise RuntimeError(line)
        self.address = next(word for word in line.split() if word.startswith('127.0.0.1:')).rstrip(',;')
        self.port = int(self.address.rsplit(':', 1)[1])
        self.pid = self.process.pid
        if trace:
            children = pathlib.Path(f'/proc/{self.pid}/task/{self.pid}/children')
            for _ in range(100):
                ids = children.read_text().split()
                if ids:
                    self.pid = int(ids[0]); break
                time.sleep(.01)

    def get(self, path):
        connection = http.client.HTTPConnection('127.0.0.1', self.port, timeout=10)
        connection.request('GET', path)
        response = connection.getresponse()
        body = response.read()
        connection.close()
        return response.status, body

    def close(self):
        if self.process.poll() is None:
            try:
                self.process.stdin.write('\n'); self.process.stdin.flush()
                self.process.wait(timeout=10)
            except (BrokenPipeError, subprocess.TimeoutExpired):
                self.process.kill(); self.process.wait()
        self.log.close()


def monitor(server, stop, samples):
    while not stop.wait(.01):
        try:
            samples.append(rss(server.pid))
        except FileNotFoundError:
            return


def measured(server, fn):
    stop = threading.Event(); samples = [rss(server.pid)]
    thread = threading.Thread(target=monitor, args=(server, stop, samples)); thread.start()
    start = time.monotonic()
    try:
        result = fn()
    finally:
        stop.set(); thread.join()
    return {'seconds': time.monotonic()-start, 'rss_min_kib': min(samples), 'rss_max_kib': max(samples), 'samples': len(samples), 'result': result}


def main():
    with tempfile.TemporaryDirectory(prefix='rust-serv-phase10-') as temp:
        root = pathlib.Path(temp)
        size = 2*1024**3
        with open(root/'big.bin', 'wb') as file:
            file.truncate(size)
        with open(root/'trace.bin', 'wb') as file:
            file.truncate(20*1024**2)
        server = Server(root)
        try:
            assert server.get('/health') == (200, b'ok\n')
            baseline = rss(server.pid)
            def transfer():
                curl = subprocess.Popen(['curl', '-fsS', f'http://{server.address}/big.bin'], stdout=subprocess.PIPE)
                digest = hashlib.sha256(); count = 0
                while block := curl.stdout.read(1024*1024):
                    digest.update(block); count += len(block)
                assert curl.wait() == 0
                assert count == size
                # Independent expected digest of sparse-file zeros, bounded memory.
                expected = hashlib.sha256(); zero = bytes(1024*1024)
                for _ in range(size//len(zero)): expected.update(zero)
                assert digest.digest() == expected.digest()
                return {'bytes': count, 'sha256': digest.hexdigest()}
            large = measured(server, transfer)
            def slow():
                curl = subprocess.Popen(['curl', '-sS', '--limit-rate', '512k', '--max-time', '6', '-o', '/dev/null', f'http://{server.address}/big.bin'], stderr=subprocess.PIPE)
                time.sleep(.3)
                before = time.monotonic(); assert server.get('/health') == (200,b'ok\n'); latency=(time.monotonic()-before)*1000
                queues = subprocess.check_output(['ss','-tnp'], text=True)
                (OUT/'slow-send-queue.txt').write_text('\n'.join(line for line in queues.splitlines() if server.address in line)+'\n')
                error = curl.communicate(timeout=10)[1].decode()
                assert curl.returncode == 28  # deliberate timeout: client disconnects mid-file
                return {'curl_exit': curl.returncode, 'curl_message': error.strip(), 'health_ms':latency}
            slow_result = measured(server, slow)
            deadline = time.monotonic()+5
            while True:
                stats = json.loads(server.get('/stats')[1])
                if stats['active'] <= 1: break  # /stats itself is the one active socket
                assert time.monotonic()<deadline
                time.sleep(.05)
            events = []
            start = time.monotonic()
            curl = subprocess.Popen(['curl','-fsS','-N',f'http://{server.address}/stream'],stdout=subprocess.PIPE,text=True)
            for line in curl.stdout:
                if line.startswith('data:'):
                    events.append({'seconds':time.monotonic()-start, 'line':line.strip()})
            assert curl.wait()==0 and len(events)==10
            assert events[0]['seconds']<1 and events[-1]['seconds']>2
            raw = subprocess.check_output(['curl','-fsS','--raw','-i',f'http://{server.address}/stream'])
            (OUT/'stream-raw.txt').write_bytes(raw)
            assert raw.endswith(b'0\r\n\r\n') and b'Transfer-Encoding: chunked' in raw and b'Content-Length:' not in raw
            result = {'baseline_rss_kib':baseline,'large':large,'slow':slow_result,'events':events,'stats_after_disconnect':stats}
            (OUT/'experiment-output.json').write_text(json.dumps(result, indent=2)+'\n')
            print(json.dumps(result, indent=2),flush=True)
        finally:
            server.close()
        traced=Server(root,trace=True)
        try:
            subprocess.run(['curl','-fsS','-o','/dev/null',f'http://{traced.address}/trace.bin'],check=True)
        finally:
            traced.close()
        capture = subprocess.run(['tcpdump','-D'],capture_output=True,text=True)
        (OUT/'tcpdump-availability.txt').write_text(capture.stdout+capture.stderr)

if __name__=='__main__':
    main()
