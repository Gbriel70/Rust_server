# Fase 4 — Routing e handlers

## Como executar

Dentro de `rust_serv`, rode `cargo run` e acesse `http://127.0.0.1:8080`.

```sh
curl -i http://127.0.0.1:8080/health
curl -I http://127.0.0.1:8080/health
curl -i -X DELETE http://127.0.0.1:8080/health
curl -i 'http://127.0.0.1:8080/hello?name=Gabriel'
curl -i --data-binary 'exemplo' http://127.0.0.1:8080/echo
```

## Organização e decisões

O fluxo agora é `parse_request → router.handle → Response → write_to/write_head_to`.

- `router.rs`: guarda `HashMap<String, HashMap<Method, Handler>>`. A primeira consulta distingue caminho inexistente de método não permitido, e o mapa interno fornece os métodos para `Allow` sem varrer outras rotas.
- `Handler = Box<dyn Fn(&Request) -> Response>`. O registro aceita funções ou closures com `impl Fn + 'static`. O builder consome e devolve `Self`.
- Repetir o mesmo método/caminho substitui o handler anterior. Há teste verificando a substituição e a ausência de métodos duplicados no `Allow`.
- `HEAD` é derivado exclusivamente de `GET`. Registrar `HEAD` explicitamente provoca pânico com uma mensagem orientando a registrar GET; esta restrição do router é documentada e testada. Um caminho que só oferece POST responde 405 a HEAD, com `Allow: POST`.
- `Allow` é ordenado alfabeticamente e inclui HEAD quando há GET. Por exemplo: `DELETE, GET, HEAD, POST, PUT`.
- O router mantém o body produzido pelo handler. O servidor escolhe `write_head_to` para HEAD; assim o Content-Length continua sendo o tamanho da representação GET. As duas formas de serialização compartilham a validação e a montagem dos headers.
- Respostas de erro a HEAD também omitem o body. Quando o parser falha antes de produzir Request, o servidor reconhece o prefixo `HEAD ` nos bytes recebidos para preservar esse comportamento.
- `Request::path()` e `query()` emprestam fatias de `target`, dividindo somente no primeiro `?`. Não alocam nem decodificam.
- O router separa o path em segmentos por `/`, decodifica cada segmento uma única vez e só então faz a consulta. `%2F`, `%5C` e `%00` são rejeitados com 400. Também rejeitamos barra invertida literal dentro de um segmento. Escape inválido ou resultado que não seja UTF-8 resulta em 400.
- `%3F` decodificado continua sendo dado do caminho; `%252F` vira o texto literal `%2F`, sem uma segunda decodificação. `+` continua sendo `+`.
- Caminhos distinguem maiúsculas de minúsculas. Barra final e barras repetidas são significativas. Não normalizamos `.` ou `..`. A query não participa da comparação da rota. Todas essas decisões têm testes.
- `routes.rs` reúne os seis handlers e `default_router()`, usado pelo executável e pelos testes de integração.
- `/hello` usa `world` quando não há `name`. Um `name` vazio produz `hello, `. A query é dividida por `&` e pelo primeiro `=` antes da decodificação. A primeira ocorrência de `name` vence; todos os componentes são validados, inclusive ocorrências posteriores e parâmetros desconhecidos.
- `/echo` preserva todos os bytes e copia o Content-Type recebido; na ausência dele, usa `application/octet-stream`.
- `Response::text` centraliza o Content-Type `text/plain; charset=utf-8`, repetido nos handlers de texto e respostas do router.
- `/counter` captura um `Cell<u32>` por `move`. Seu estado pertence à instância do router. HEAD executa o mesmo handler e também incrementa o contador. O contador é um exercício em memória, sem persistência; o limite de u32 não foi expandido nesta fase.
- O servidor continua sequencial e fecha a conexão depois de responder. Os handlers ainda não exigem Send ou Sync.

## Validação

Foram aprovados **85 testes**, incluindo **19 testes do router**, a tabela com **22 casos de percent_decode** e os **13 cenários novos de integração**. Os testes da Fase 3 continuam presentes; o POST válido foi adaptado para `/echo` e a raiz agora retorna `hello\n`.

O helper em `tests/support/mod.rs` cria router e servidor dentro da thread que atende. Ele envia apenas o SocketAddr à thread do teste, por um canal. Isso evita exigir Send dos handlers. Cada teste usa porta 0 e timeouts de conexão, leitura, escrita e recebimento do endereço. O helper lê até EOF e verifica os bytes após o terminador HTTP: HEAD precisa ter body vazio; outras respostas precisam ter body do tamanho anunciado. A comparação entre GET e HEAD também verifica os headers completos.

- `cargo test`: passou; [saída completa](rust_serv/validation/phase4/cargo-test.txt).
- `cargo test -q`, 20 execuções consecutivas: **20/20 passaram**; [saída do loop](rust_serv/validation/phase4/test-loop.txt).
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` e `cargo build --release`: passaram; [saída](rust_serv/validation/phase4/checks.txt).
- Em uma cópia temporária do projeto, removemos apenas `allowed.sort_unstable()`: o teste do Allow falhou **20/20 vezes**, mostrando ordens diferentes, como `DELETE, PUT, GET, POST, HEAD` e `POST, PUT, GET, DELETE, HEAD`. Nesse teste HEAD é acrescentado no fim antes da ordenação, então a ausência da ordenação sempre viola a sequência esperada com vários métodos. A ordem dos demais métodos também variou. [Saída da experiência](rust_serv/validation/phase4/unsorted-allow.txt).
- Os testes com sockets foram executados fora do sandbox, com autorização.

As saídas estão em `validation/phase4/`. A pasta `validation` já é ignorada pelo Git: esses artefatos locais precisam ser enviados separadamente se forem usados numa revisão externa.

## Experimentos realizados

Foram usados o router e o servidor reais da biblioteca, com um executável temporário que apenas faz bind na porta 0 e imprime o endereço. Isso evita conflito com serviços na porta 8080. [Registro dos experimentos](rust_serv/validation/phase4/experiments.txt).

1. **Todas as rotas com curl:** `/`, `/health`, `/headers`, `/hello`, `/counter` e POST `/echo` responderam 200. GET `/health` retornou `ok\n`. `curl -I /health` retornou exatamente os mesmos headers que GET, incluindo `Content-Length: 3`, sem o body.

2. **405 versus 501:** DELETE `/health` retornou 405 com `Allow: GET, HEAD`, produzido pelo router. BREW `/health` retornou 501 com `unsupported method`, produzido a partir do erro do parser.

3. **Dot-segments:** o curl normal enviou `GET /health HTTP/1.1` ao receber a URL `/a/../health`, resultando em 200. Com `--path-as-is`, enviou `GET /a/../health HTTP/1.1`, resultando em 404. As linhas efetivamente enviadas foram registradas por `curl -v`.

4. **Percent-encoding:** `/caf%C3%A9` retornou 404 porque o router padrão não registra `/café`; a correspondência com uma rota Unicode registrada foi verificada em teste unitário. `/a%2Fb`, `/x%00`, `/%zz` e `/%4` retornaram 400.

5. **Round-trip binário:** 100.000 bytes aleatórios foram enviados para `/echo`; `cmp` retornou código 0. O teste de integração também enviou os 256 valores possíveis de byte. Um arquivo com 1.048.577 bytes, acima de MAX_BODY, recebeu 413. Nesse experimento usamos `Expect: 100-continue` no curl para que o servidor pudesse rejeitar o Content-Length antes do upload do body, evitando o problema de fechar com dados não lidos.

6. **HEAD em bytes crus:** uma conexão TCP confirmou que a resposta termina em `\r\n\r\n`, apesar de anunciar Content-Length 3. Como `xxd` não está instalado, registramos os bytes em hexadecimal com Python. Num servidor experimental isolado, enviamos deliberadamente `ok\n` depois do head de HEAD; o curl com `-I` ainda exibiu só os headers e terminou com sucesso. Portanto, a saída visual do curl não basta para provar a ausência do body; os testes de bytes crus detectam isso.

7. **Syscalls:** `strace -f -e trace=sendto` registrou **uma chamada de 101 bytes para GET /health** e **uma chamada de 98 bytes para HEAD /health**. A diferença corresponde a `ok\n`. [Trace](rust_serv/validation/phase4/strace-get-head.txt). Uma escrita pode ser parcial em outras condições; os números são observações desta execução.

   **Pendência: captura de pacotes.** O tcpdump, limitado à porta efêmera do experimento, foi bloqueado com `You don't have permission to perform this capture on that device`. A tentativa com `sudo -n` também não foi possível: `sudo: uma senha é necessária`. Não houve captura TCP. Para completar esta parte manualmente, com o servidor na porta 8080, rode `sudo tcpdump -i lo -nn -A 'tcp port 8080'` e faça um GET e um HEAD em outro terminal. A comparação por strace e pelos bytes recebidos foi concluída.

8. **Browser:** abrimos `/headers` e `/hello?name=Gabriel` no Chrome headless. O browser exibiu `hello, Gabriel` e enviou headers adicionais como `sec-ch-ua`, `Sec-Fetch-*`, `Accept-Language`, `Accept-Encoding` e `Connection: keep-alive`; o servidor respondeu com `Connection: close`. O curl enviou um conjunto menor de headers. O monitor de rede do Chrome registrou automaticamente `/favicon.ico` com **404** nas duas navegações. Registros: [headers](rust_serv/validation/phase4/browser-headers.json) e [hello](rust_serv/validation/phase4/browser-hello.json).

9. **Estado:** três conexões à mesma instância retornaram `1`, `2`, `3`. Após encerrar e iniciar outro processo, a primeira resposta voltou a `1`.

## Exercícios de compilação

- Começamos com `type Handler = fn(&Request) -> Response`. Ao registrar o contador com estado capturado, o compilador produziu E0308: `expected fn pointer, found closure`, explicando que só closures sem capturas podem ser convertidas em fn pointers. A passagem para Box<dyn Fn> foi feita depois dessa observação. [Mensagem](rust_serv/validation/phase4/function-pointer-error.txt).
- A versão explícita `fn path<'a>(&'a self) -> &'a str` compilou e retornou `/a` para `/a?b=c`. Ela expressa o mesmo vínculo de empréstimo que a versão com lifetimes elididos. [Exercício](rust_serv/validation/phase4/lifetime-explicit.txt).
- `fn longest(a: &str, b: &str) -> &str` produziu E0106: falta especificar de qual empréstimo a saída depende. Uma assinatura possível é `fn longest<'a>(a: &'a str, b: &'a str) -> &'a str`. [Mensagem](rust_serv/validation/phase4/lifetime-error.txt).
- Mutar diretamente um contador capturado numa closure exigida como Fn produziu E0594. Cell permite atualizar seu conteúdo por uma referência compartilhada. [Mensagem](rust_serv/validation/phase4/fn-mut-error.txt).
- Mover o router pronto para `thread::spawn` produziu E0277 porque `dyn Fn(&Request) -> Response` não inclui Send. Esse é o erro imediato observado; não foi um erro diretamente sobre Cell. `Cell<u32>` pode ser movido entre threads, mas não compartilhado como Sync. Na fase de concorrência, a forma de compartilhar estado precisará ser reconsiderada. [Mensagem](rust_serv/validation/phase4/router-send-error.txt).

## Respostas às nove perguntas

1. **Por que o router aparece agora?** Até a Fase 3, uma função que devolvia a mesma resposta atendia a necessidade. Agora existem caminhos e métodos com comportamentos diferentes; repetir condicionais no servidor deixaria o transporte responsável pelo despacho da aplicação. O router passa a ter um problema concreto para resolver.

2. **Por que impl Fn + 'static na entrada e Box<dyn Fn> no armazenamento?** Cada closure tem seu próprio tipo. O parâmetro genérico aceita todos eles, e o trait object permite guardá-los no mesmo mapa e chamá-los pela mesma interface. `'static` impede capturar referências locais de vida curta. Não obriga o handler a viver para sempre: dados capturados por valor com `move` são liberados junto com o router.

3. **Por que Cell com Fn?** Fn chama a closure por referência compartilhada. Alterar diretamente uma variável capturada exige FnMut. Cell oferece mutabilidade interior e permite manter `Router::handle(&self)`. Com FnMut, o mapa guardaria Box<dyn FnMut>, a consulta usaria get_mut e handle precisaria receber `&mut self`.

4. **Quando usar 404, 405 e 501?** 404 significa caminho sem rota. 405 significa caminho registrado que não aceita aquele método conhecido. 501 é usado aqui para um método que o parser não suporta: BREW falha antes de chegar ao router; DELETE é reconhecido, mas não está registrado em `/health`.

5. **Por que Allow no 405?** Informa quais métodos aquele recurso aceita, permitindo ao cliente escolher uma operação suportada. HEAD aparece automaticamente porque o router pode executar GET e o servidor enviar apenas seus headers.

6. **Por que dividir antes de decodificar?** `%3F` e `%2F` podem representar dados. Se a decodificação acontecesse antes da divisão, eles virariam delimitadores estruturais e poderiam alterar a rota ou iniciar uma query inesperada. Dividimos primeiro e rejeitamos separadores codificados dentro dos segmentos de path. Na query, dividimos por `&` e `=` antes de decodificar pelo mesmo motivo.

7. **Quem normaliza `/a/../b`?** No experimento, o curl normalizou antes de enviar. `--path-as-is` preservou o caminho original. Nosso servidor não normaliza dot-segments nesta fase, e por isso `/a/../health` recebido literalmente resultou em 404.

8. **Por que o servidor suprime o body de HEAD?** O handler produz a representação e seus headers uma única vez, e o transporte escolhe quais bytes enviar. Apagar o body antes de serializar faria o Content-Length virar zero; a serialização só do head preserva o comprimento correto. A regra também se aplica às respostas de erro.

9. **Por que Hash e ordenação?** Method é chave de HashMap e precisa implementar Hash junto com Eq. A iteração de HashMap não fornece ordem estável. Ordenar os nomes produz um Allow determinístico, legível e testável sem depender da semente aleatória do mapa.

---

# Fase 5 — Conexões persistentes e limites

## Implementação e decisões

O buffer pertence a `Connection` em `rust_serv/src/connection.rs`. O servidor mantém o loop de requests e a política de persistência; a conexão cuida de leitura, escrita, timeouts e fechamento TCP.

`Server::bind(addr, router)` continua usando a configuração padrão. Para configurar limites, use:

```rust
use std::time::Duration;
use rust_serv::{connection::Config, routes, server::Server};

let config = Config {
    idle_timeout: Duration::from_millis(200),
    request_timeout: Duration::from_millis(500),
    ..Config::default()
};
let server = Server::bind_with_config("127.0.0.1:8080", routes::default_router(), config)?;
server.run()?;
```

Os padrões são **5 s de ociosidade, 10 s para completar uma request, 10 s por escrita bloqueada e 100 requests por conexão**. Config é Copy. Timeouts zero e max_requests zero são rejeitados com InvalidInput antes do bind; Connection::new também valida a configuração. O timeout de escrita é aplicado na criação da conexão.

- **Parser antes do socket:** se o buffer já contém uma request completa, ela é devolvida imediatamente. `Vec::drain(..consumed)` remove só os bytes dessa request, preservando o restante.
- **Deadline total:** começa na primeira leitura que trouxe bytes da request. O tempo restante é calculado com saturating_sub, sem renovar o prazo quando mais bytes chegam. Zero vira RequestTimeout diretamente, sem passar Duration::ZERO ao socket.
- **Bytes antecipados:** uma request parcial que chegou junto da anterior conserva o horário daquela leitura, inclusive durante a execução do handler anterior. Como fazemos parse antes de cada leitura, qualquer sobra após uma request completa veio da última leitura do socket. Esse horário é guardado em `last_read_at`. Uma request já completa no buffer é processada antes de verificar timeout.
- **Ociosidade:** o prazo ocioso começa ao esperar a próxima request com buffer vazio. Tempo passado ocioso não é descontado do prazo da nova request. Interrupted repete a operação com o tempo restante; WouldBlock e TimedOut são traduzidos para IdleTimeout ou RequestTimeout conforme já existam bytes.
- **HTTP/1.1:** persiste por padrão. HTTP/1.0 precisa de `Connection: keep-alive`, que também é anunciado na resposta. `close` sempre vence. O token é procurado em todas as linhas Connection, separando por vírgula, removendo espaços/tabulações e comparando sem distinguir maiúsculas.
- **Limite de requests:** a última resposta permitida anuncia `Connection: close`. Bytes de requests posteriores não são roteados.
- **Erros:** todo erro do parser encerra a conexão depois de responder. RequestTimeout responde 408 e fecha; IdleTimeout fecha silenciosamente. EOF parcial e erros de I/O retornam ao loop principal para log, sem gerar outra resposta HTTP. O servidor também fecha respostas 400, 408, 413 e 431 produzidas pelo router/handler; 404 e 405 podem continuar na conexão.
- **HEAD:** continua omitindo o body, incluindo nos erros e no 408, mas anuncia o tamanho que seria enviado. Os bytes da resposta seguinte permanecem alinhados.
- **Lingering:** depois de escrever uma resposta final, `linger_close` chama shutdown(Write), descarta o restante do buffer já lido e drena o socket por **até 64 KiB ou 1 segundo**, o que ocorrer primeiro. O prazo é total, mesmo quando o cliente continua enviando. Falhas no dreno encerram essa tentativa; ele não substitui o erro original de escrita. Não é implementado em Drop.
- **Limite da garantia:** drenar até esses tetos resolve a bomba de 20 KB testada. Um cliente que ultrapasse o teto de bytes ou de tempo ainda pode provocar fechamento com entrada não lida; o servidor não promete drenar uma quantidade ilimitada.
- **Expect:** nenhum 100 Continue foi implementado, conforme o escopo. O parser ainda entrega a request somente depois do body completo.

## Testes e saídas

**113 testes passaram**, incluindo 17 combinações na tabela de keep_alive, testes de has_token, **20 testes de integração de persistência** cobrindo todos os 17 cenários do roteiro, e 6 testes de Connection/Config.

O helper `tests/support/mod.rs` agora lê uma resposta por vez: encontra o terminador do head, interpreta Content-Length, lê zero bytes de body para HEAD e preserva os bytes restantes para a próxima resposta. Não depende do fechamento para terminar uma resposta persistente. A verificação de EOF exige leitura zero, e rejeita ConnectionReset. Nos testes anteriores, max_requests é explicitamente 1 para preservar as verificações de resposta única e fechamento.

Além dos cenários pedidos, os testes verificam timeout no body, HEAD incompleto com 408 sem body, renovação do prazo entre requests, chegada antecipada de uma request parcial, requests completas já no buffer, POST binário seguido de GET e continuidade depois de 404/405. O teste de escrita usa uma resposta de 16 MiB e um cliente que não lê, para exceder o buffer do socket e exercitar o timeout; o limite do parser para bodies de requests permanece 1 MiB.

- [cargo test](rust_serv/validation/phase5/cargo-test.txt): passou.
- [Loop completo](rust_serv/validation/phase5/test-loop.txt): **20/20 execuções passaram**. Cada execução testa a bomba de headers em 20 conexões, totalizando **400 fechamentos confiáveis** nesse loop.
- [fmt, clippy e release](rust_serv/validation/phase5/checks.txt): `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` e `cargo build --release` passaram.
- [Teste de sanidade sem lingering](rust_serv/validation/phase5/without-linger.txt): numa cópia temporária, removemos a chamada a linger_close. O teste da bomba falhou **20/20 vezes**, com `Connection reset by peer`. O reset foi detectado ao verificar o encerramento, depois de ler a resposta; portanto, nesta medição ele não implicou perda de todos os bytes da resposta.

A pasta `rust_serv/validation` é ignorada pelo Git. Os logs são artefatos locais e precisam ser enviados separadamente numa revisão externa. Os testes com sockets foram executados fora do sandbox, com autorização.

## Experimentos observados

Os experimentos usaram o servidor real da biblioteca, com um executável temporário que apenas seleciona Config, faz bind na porta 0 e imprime o endereço. [Registro completo](rust_serv/validation/phase5/experiments.txt).

1. **Reuso:** curl com duas URLs informou reutilização da conexão. Com 100 URLs, a soma de `%{num_connects}` foi **1**; com 100 processos curl separados, foi **100**. Um trace completo e isolado mostrou **1 accept4 e 100 sendto** na execução de 100 requests. O `ss` mostrou **1 conexão ESTABLISHED** enquanto uma conexão persistente estava aberta. O primeiro recorte de strace começava no meio de uma chamada accept4 e não servia para contá-la; usamos o trace completo da execução isolada para a contagem. [Trace completo](rust_serv/validation/phase5/timewait-reuso-strace.txt).

2. **Pipelining:** duas requests foram enviadas numa única escrita TCP. As respostas chegaram na ordem: `hello\n`, depois `ok\n`. A segunda request pediu Connection: close, permitindo também verificar EOF. O teste de integração cobre uma terceira request dividida entre escritas.

3. **Servidor sequencial bloqueado:** uma conexão ociosa foi aberta antes de um curl. Com idle_timeout de 5 s, o curl aguardou **5,115 s** até receber `ok\n`. O valor inclui a resolução do timeout do kernel e o escalonamento; não é uma promessa de temporização exata.

4. **Slowloris:** enviando linhas a cada 100 ms, com request_timeout de 500 ms, o servidor respondeu **408 após 0,502 s**. O fluxo de bytes não renovou o prazo.

5. **Half-close:** o cliente enviou uma request e chamou shutdown(Write). Recebeu a resposta 200 completa e depois EOF. O teste também cobre duas requests completas no buffer antes do half-close.

6. **RST/fechamento:** a bomba de aproximadamente 20 KB recebeu 431 e a leitura terminou com EOF, sem reset. Na cópia sem lingering, a verificação de encerramento encontrou reset em todas as 20 tentativas. As flags FIN/RST dos pacotes não foram capturadas, pela limitação de permissão abaixo.

7. **Expect: 100-continue:** com curl 7.88.1 e um body de 1 MiB, a chamada comum não incluiu Expect automaticamente e levou **0,048711 s**. Com `-H 'Expect:'`, levou **0,049131 s**. Forçando `-H 'Expect: 100-continue'`, levou **1,053260 s**, e o curl registrou `Done waiting for 100-continue` antes de enviar o body. A diferença observada foi aproximadamente 1 s. O resultado de uma única execução não é um benchmark de desempenho.

8. **Syscalls:** o strace mostrou SO_SNDTIMEO na criação da conexão, SO_RCVTIMEO antes das leituras e o prazo de leitura diminuindo durante slowloris. Após o 408, observamos `sendto → shutdown(SHUT_WR) → recvfrom → close`. Na conexão que ficou ociosa depois de um GET, recvfrom retornou **EAGAIN após cerca de 200 ms**, seguido de close, sem resposta adicional. [Trace](rust_serv/validation/phase5/timeouts-strace.txt).

9. **TIME_WAIT:** no experimento com curl e Connection: close em 100 processos, o snapshot mostrou **84 entradas do lado servidor**; na execução de 100 URLs em um curl, mostrou **0** desse lado. Isso não deve ser substituído pela expectativa teórica: curl pode fechar ao terminar o body, antes de receber o FIN, e o lado que inicia o fechamento pode variar. Fizemos um controle em que o cliente espera EOF antes de fechar seu lado de escrita: foram **100 TIME_WAIT para 100 conexões** e **1 TIME_WAIT para uma conexão com 100 requests**. [Medição controlada](rust_serv/validation/phase5/timewait-controlled.txt).

10. **Browser:** Chrome headless carregou `/` e fez três fetches de `/health`, todos com **Connection ID 97**, confirmando o reuso. A navegação numa segunda aba levou **0,008 s** com idle_timeout de 500 ms; o travamento entre abas não apareceu nessa execução. O bloqueio sequencial foi reproduzido de forma controlada no experimento 3. [Eventos de rede](rust_serv/validation/phase5/browser-network.json).

**Pendência dos experimentos 1 e 6:** tcpdump retornou `You don't have permission to perform this capture on that device (socket: Operation not permitted)`. Não contamos SYNs nem inspecionamos flags FIN/RST nos pacotes. A necessidade de senha de sudo já havia sido constatada na Fase 4; não foi solicitada senha pelo chat. Curl, ss, strace e os testes de sockets fornecem as evidências acima, mas não substituem uma captura de pacotes.

Para completar a captura manualmente com o servidor na porta 8080:

```sh
# Num terminal com permissão para capturar:
sudo tcpdump -i lo -nn 'tcp[tcpflags] & (tcp-syn|tcp-ack) == tcp-syn and port 8080'
# Para observar FIN/RST, capture todo o tráfego dessa porta:
sudo tcpdump -i lo -nn 'tcp port 8080'
```

O experimento opcional de Nagle não foi executado.

## Respostas às onze perguntas

1. **TCP keep-alive versus HTTP keep-alive:** TCP keep-alive usa sondas para detectar uma conexão sem resposta no nível de transporte. HTTP keep-alive reutiliza a mesma conexão para várias requests/responses. Implementamos o segundo; não ativamos SO_KEEPALIVE.

2. **Por que tentar o parser primeiro?** Uma leitura pode trazer várias requests. Depois de responder à primeira, a segunda pode estar completa no Vec. Ler antes de verificar esse Vec faz o servidor esperar bytes adicionais enquanto o cliente espera a resposta que ele já poderia produzir.

3. **Por que deadline total?** Um timeout por leitura começa de novo a cada read. Um cliente que envia poucos bytes periodicamente evita esse timeout indefinidamente. O deadline total não é renovado, então a request incompleta recebe 408 mesmo com atividade contínua.

4. **Ociosidade versus request incompleta:** sem bytes de uma nova request, não há request pendente à qual responder; fechamos em silêncio para liberar o servidor. Com uma request iniciada que não termina no prazo, respondemos 408 e fechamos.

5. **Por que fechar depois do erro de parse?** Não podemos confiar nos limites da mensagem para encontrar a próxima request. No 413 por Content-Length, por exemplo, o body declarado ainda pode estar todo no socket. Interpretá-lo como outra request permitiria confusão de mensagens. O dreno descarta esses bytes sem roteá-los.

6. **O que lingering muda?** Shutdown da escrita inicia o fechamento da direção servidor→cliente. Manter a leitura ativa e consumir os dados pendentes permite concluir o fechamento sem abandonar imediatamente entrada não lida, situação que pode causar RST. O dreno tem limite de bytes e de tempo para que um cliente não prenda o servidor enviando dados para sempre. Depois desses limites, o fechamento ainda precisa ocorrer.

7. **Por que o servidor sequencial piora?** O loop atende uma conexão inteira antes de aceitar a próxima. Agora uma conexão pode ficar esperando outra request até idle_timeout, mesmo sem trabalho útil, enquanto outros clientes aguardam. A Fase 7 introduzirá concorrência para enfrentar esse bloqueio.

8. **Quem acumula TIME_WAIT e como reduzir?** Em um fechamento TCP normal, o lado que inicia o fechamento ativo costuma ficar em TIME_WAIT; fechamento simultâneo e temporização podem alterar o lado observado. Reusar uma conexão reduz o número de conexões que precisam ser encerradas. O controle esperando EOF mostrou 100 entradas contra 1.

9. **Por que body em HEAD desalinha?** O cliente sabe que HEAD não tem body, independentemente do Content-Length anunciado. Se o servidor enviar esse body mesmo assim, seus bytes serão tratados como o início da próxima status line. O teste HEAD seguido de GET verifica esse alinhamento diretamente.

10. **Por que Config e Connection agora?** O buffer precisa sobreviver entre requests para guardar sobras. Os timeouts e max_requests precisam de valores configuráveis, especialmente para testes rápidos. Na fase de uma única request, a variável local de buffer era suficiente.

11. **Por que método e não Drop?** Lingering realiza I/O e pode bloquear até o prazo de drenagem. Uma chamada explícita torna esse custo visível no fluxo que envia a resposta final. Drop permanece responsável apenas pela liberação normal do stream, sem introduzir uma espera escondida em qualquer caminho de saída.
