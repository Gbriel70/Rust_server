'use strict';
const element = (id) => document.getElementById(id);
let socket;
function log(message) {
  const lines = element('log').textContent.split('\n').filter(Boolean);
  lines.push(message);
  element('log').textContent = lines.slice(-40).join('\n') + '\n';
}
function state(message) {
  element('status').textContent = message;
  const open = socket?.readyState === WebSocket.OPEN;
  element('connect').disabled = Boolean(socket && socket.readyState !== WebSocket.CLOSED);
  for (const id of ['send', 'binary', 'close']) element(id).disabled = !open;
}
element('connect').addEventListener('click', () => {
  const url = new URL('/ws', location.href);
  url.protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
  socket = new WebSocket(url);
  socket.binaryType = 'arraybuffer';
  state('Conectando…');
  socket.addEventListener('open', () => { state('Conectado'); log('Conexão WebSocket aberta'); });
  socket.addEventListener('message', (event) => {
    log(typeof event.data === 'string' ? `Recebido · ${event.data}` : `Bytes recebidos · ${Array.from(new Uint8Array(event.data)).join(', ')}`);
  });
  socket.addEventListener('close', (event) => {
    state(`Desconectado · código ${event.code}`);
    log(`Fechamento · ${event.code} · ${event.wasClean ? 'concluído' : 'conexão interrompida'}`);
  });
  socket.addEventListener('error', () => log('Falha na conexão. Confira se o servidor está disponível.'));
});
element('message-form').addEventListener('submit', (event) => {
  event.preventDefault();
  if (socket?.readyState !== WebSocket.OPEN) return;
  socket.send(element('message').value);
  log(`Enviado · ${element('message').value}`);
});
element('binary').addEventListener('click', () => {
  if (socket?.readyState !== WebSocket.OPEN) return;
  socket.send(new Uint8Array([0, 1, 2, 255]));
  log('Bytes enviados · 0, 1, 2, 255');
});
element('close').addEventListener('click', () => {
  socket.close(1000, 'Fim do experimento');
  state('Fechando…');
});
