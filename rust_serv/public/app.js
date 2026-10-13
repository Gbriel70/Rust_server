'use strict';

const $ = (id) => document.getElementById(id);
const experiments = {
  hello: { title: 'Uma resposta com o seu nome.', concept: 'PARÂMETROS NA URL', description: 'O método GET pede um recurso. O parâmetro name leva seu nome na URL até o handler da rota /hello.', label: 'Como você se chama?', value: 'Visitante', path: (value) => `/hello?name=${encodeURIComponent(value)}`, expected: 200, note: 'Espere um status 200 e a saudação no corpo. Troque o nome e envie novamente.' },
  echo: { title: 'Sua mensagem, de volta para você.', concept: 'CORPO DA REQUISIÇÃO', description: 'POST envia dados no corpo da requisição. A rota /echo devolve exatamente os bytes que recebeu.', label: 'Qual mensagem você quer enviar?', value: 'Olá, servidor! Esta mensagem fez uma viagem por HTTP.', method: 'POST', path: () => '/echo', expected: 200, note: 'A resposta deve repetir sua mensagem. Acentos também viajam como bytes; Content-Length conta bytes, não letras.' },
  headers: { title: 'Uma mensagem leva mais que conteúdo.', concept: 'HEADERS', description: 'Headers são informações sobre a requisição. Este teste acrescenta X-Lab: rust-server e mostra o que o servidor recebeu.', path: () => '/headers', headers: { 'X-Lab': 'rust-server' }, expected: 200, note: 'Procure x-lab no corpo. Outros headers foram acrescentados pelo navegador. À direita estão os headers da resposta, que são outra mensagem.' },
  head: { title: 'Conheça o arquivo sem baixar o corpo.', concept: 'MÉTODO HEAD', description: 'HEAD pede os metadados que acompanham um GET. Aqui consultamos o HTML desta página.', method: 'HEAD', path: () => '/index.html', expected: 200, note: 'O corpo fica vazio, mas Content-Length descreve o tamanho do arquivo que seria enviado em um GET.' },
  range: { title: 'Só os primeiros cem bytes.', concept: 'RESPOSTA PARCIAL', description: 'O header Range: bytes=0-99 solicita um trecho do HTML desta página. Os índices começam em zero.', path: () => '/index.html', headers: { Range: 'bytes=0-99' }, expected: 206, note: 'Espere 206 Partial Content, Content-Length: 100 e Content-Range mostrando o intervalo e o tamanho total.' },
  counter: { title: 'Uma rota que se lembra das visitas.', concept: 'ESTADO COMPARTILHADO', description: 'Cada chamada a /counter incrementa um contador atômico compartilhado pelos handlers deste processo.', path: () => '/counter', expected: 200, note: 'Envie mais de uma vez e veja o número crescer. Outros visitantes também o incrementam; reiniciar o servidor reinicia o contador.' },
  missing: { title: 'Nem todo pedido encontra um recurso.', concept: 'ERRO 404', description: 'Vamos solicitar um arquivo inexistente. O servidor deve responder com um erro HTTP, sem deixar de atender.', path: () => '/arquivo-inexistente-laboratorio.txt', expected: 404, note: '404 Not Found é o resultado esperado deste teste. Uma resposta de erro também tem status, headers e corpo.' },
  method: { title: 'Mesmo caminho, método diferente.', concept: 'ERRO 405', description: 'A rota /echo aceita POST. Este experimento usa GET de propósito para observar a validação do router.', path: () => '/echo', expected: 405, note: 'Espere 405 Method Not Allowed. O header Allow indica POST, o método aceito pela rota.' },
};
let selected = 'hello';
let busy = false;
const values = { hello: experiments.hello.value, echo: experiments.echo.value };

function preview() {
  const test = experiments[selected];
  $('request-method').textContent = test.method || 'GET';
  $('request-path').textContent = test.path($('request-input').value);
}
function selectExperiment(key) {
  if (busy) return;
  if (experiments[selected].label) values[selected] = $('request-input').value;
  selected = key;
  const test = experiments[key];
  document.querySelectorAll('[data-test]').forEach((button) => button.setAttribute('aria-pressed', String(button.dataset.test === key)));
  $('experiment-title').textContent = test.title;
  $('concept').textContent = test.concept;
  $('experiment-description').textContent = test.description;
  $('expectation').textContent = test.note;
  $('input-group').hidden = !test.label;
  $('input-label').textContent = test.label || '';
  $('request-input').value = values[key] || '';
  $('request-status').textContent = 'Pronto para testar · resposta esperada: ' + test.expected;
  $('response-body').textContent = 'Envie a requisição para ver o resultado deste experimento.';
  $('response-headers').textContent = 'Os headers da resposta aparecerão aqui.';
  preview();
}
document.querySelectorAll('[data-test]').forEach((button) => button.addEventListener('click', () => selectExperiment(button.dataset.test)));
$('request-input').addEventListener('input', preview);

// Um prazo finito permite recuperar os controles mesmo se o servidor parar.
async function request(path, options = {}) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 15000);
  try {
    const response = await fetch(path, { ...options, cache: 'no-store', signal: controller.signal });
    const body = await response.text();
    return { response, body };
  } finally { clearTimeout(timer); }
}
function failure(error) {
  return error.name === 'AbortError' ? 'Tempo de espera esgotado. Confira se o servidor está rodando e tente novamente.' : 'Não foi possível acessar o servidor. Confira a conexão e abra esta página pelo endereço HTTP do servidor.';
}
$('request-form').addEventListener('submit', async (event) => {
  event.preventDefault();
  if (busy) return;
  busy = true;
  const test = experiments[selected];
  const method = test.method || 'GET';
  const buttons = [$('send'), ...document.querySelectorAll('[data-test]')];
  buttons.forEach((button) => { button.disabled = true; });
  $('request-status').textContent = 'Enviando requisição…';
  $('response-body').textContent = 'Aguardando o servidor…';
  $('response-headers').textContent = 'Aguardando headers…';
  const start = performance.now();
  try {
    const { response, body } = await request(test.path($('request-input').value), { method, headers: method === 'POST' ? { 'Content-Type': 'text/plain; charset=utf-8' } : test.headers, ...(method === 'POST' ? { body: $('request-input').value } : {}) });
    const elapsed = Math.round(performance.now() - start);
    $('request-status').textContent = `${response.status} ${response.statusText} · ${elapsed} ms · ${response.status === test.expected ? 'resultado esperado' : 'resultado diferente do esperado'}`;
    $('response-body').textContent = body || (method === 'HEAD' ? '(Sem corpo: este é o comportamento de HEAD.)' : '(Resposta sem corpo.)');
    $('response-headers').textContent = Array.from(response.headers, ([name, value]) => `${name}: ${value}`).join('\n');
  } catch (error) {
    $('request-status').textContent = 'Falha na conexão';
    $('response-body').textContent = failure(error);
    $('response-headers').textContent = 'Nenhuma resposta HTTP recebida.';
  } finally {
    busy = false;
    buttons.forEach((button) => { button.disabled = false; });
  }
});

let stream = null;
let streamTimer = null;
function stopStream(message) {
  if (stream) stream.close();
  stream = null;
  clearTimeout(streamTimer);
  $('start-stream').disabled = false;
  $('stop-stream').disabled = true;
  $('stream-status').textContent = message;
}
$('start-stream').addEventListener('click', () => {
  if (stream) return;
  let count = 0;
  $('stream-output').textContent = '';
  $('stream-status').textContent = 'Conectando a /stream…';
  $('start-stream').disabled = true;
  $('stop-stream').disabled = false;
  stream = new EventSource('/stream');
  streamTimer = setTimeout(() => stopStream('Tempo esgotado. Você pode tentar novamente.'), 15000);
  stream.onmessage = (event) => {
    count += 1;
    $('stream-output').textContent += `${String(count).padStart(2, '0')}  ←  ${event.data}\n`;
    $('stream-output').scrollTop = $('stream-output').scrollHeight;
    $('stream-status').textContent = `${count} de 10 eventos recebidos`;
    if (count === 10) stopStream('Concluído: 10 eventos recebidos pela mesma conexão.');
  };
  // EventSource tentaria reconectar automaticamente. A demonstração é finita.
  stream.onerror = () => stopStream(`Conexão interrompida após ${count} eventos. Confira o servidor e tente novamente.`);
});
$('stop-stream').addEventListener('click', () => stopStream('Interrompido por você. Clique em iniciar para repetir.'));
window.addEventListener('pagehide', () => { if (stream) stopStream('Conexão encerrada ao sair da página.'); });

$('stats-button').addEventListener('click', async () => {
  $('stats-button').disabled = true;
  $('stats-status').textContent = 'Consultando…';
  try {
    const { response, body } = await request('/stats');
    if (!response.ok) throw new Error('HTTP ' + response.status);
    $('stats-output').textContent = JSON.stringify(JSON.parse(body), null, 2);
    $('stats-status').textContent = 'Atualizado às ' + new Date().toLocaleTimeString('pt-BR') + ' · contadores do processo atual';
  } catch (error) {
    $('stats-status').textContent = 'Consulta não concluída';
    $('stats-output').textContent = failure(error);
  } finally { $('stats-button').disabled = false; }
});
