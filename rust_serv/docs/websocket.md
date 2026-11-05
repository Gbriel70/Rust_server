# WebSocket — Fase 13

## Executar e experimentar

A partir de `rust_serv/`, inicie o servidor e abra a demonstração em [http://127.0.0.1:8080/websocket.html](http://127.0.0.1:8080/websocket.html).

```bash
cargo run --locked -- --root ./public --mode tokio
```

Clique em Conectar, envie texto ou bytes e feche a conexão. O painel Network do navegador mostra o upgrade HTTP e os frames. O endpoint `/ws` devolve mensagens completas de texto e binário, incluindo mensagens vazias.

Os modos `thread`, `pool` e `epoll` também atendem `/ws`. No epoll, `--trigger level` e `--trigger edge` selecionam LT e ET. O modo epoll requer Linux.

## Arquitetura

- `src/websocket/mod.rs` valida a abertura e calcula Sec-WebSocket-Accept. SHA-1, fornecido por `sha1`, faz parte desse cálculo definido pela RFC. `base64` codifica e valida os valores do handshake.
- `src/websocket/codec.rs` decodifica frames de cliente incrementalmente e serializa frames de servidor.
- `src/websocket/session.rs` reconstitui mensagens e aplica echo, ping/pong e fechamento, com independência de sockets e runtimes.
- `src/websocket/transport.rs` mantém os buffers, offsets, deadlines e o progresso compartilhado pelos adaptadores.
- `Body::WebSocket`, junto do status 101, sinaliza a transferência do socket para a sessão. Os handlers HTTP mantêm sua assinatura de retorno `Response`.

O servidor escreve a resposta 101 inteira antes de enviar frames. Bytes que chegaram após a requisição HTTP acompanham a transferência. Os status 101 e 426 foram acrescentados ao tipo `Status`. A serialização do 101 suprime body, Content-Length e Transfer-Encoding.

O parser HTTP permanece com a implementação anterior. A rota `/ws` acrescenta validações próprias de handshake. HEAD recebe 400 nessa rota e os demais métodos recebem 405 pelo router. Versão WebSocket incompatível recebe 426 com a versão 13 anunciada. Aberturas inválidas recebem 400. As ofertas de extensões e subprotocolos seguem sem seleção na resposta.

## Frames e mensagens

O cliente envia frames mascarados e o servidor envia frames sem máscara. O codec exige comprimentos com a representação mínima, RSV zerado e opcodes reconhecidos. Frames de controle têm FIN e payload de até 125 bytes.

Texto pode chegar dividido entre frames, inclusive no meio de um caractere UTF-8. A validação do texto ocorre ao concluir a mensagem. Ping pode aparecer entre fragmentos e recebe Pong com o mesmo payload. Pong recebido dispensa resposta.

Close aceita payload vazio ou código e razão válidos. O servidor responde ao close do cliente e encerra após completar a escrita. Em um fechamento iniciado pelo servidor, ele aguarda a resposta do cliente dentro do prazo de close. Mensagens de dados posteriores ao início do fechamento são descartadas.

| Situação | Código de fechamento |
| --- | --- |
| Demonstração encerrada pelo navegador | 1000 |
| Shutdown, ociosidade ou frame incompleto fora do prazo | 1001 |
| Erro de framing ou sequência de fragmentação | 1002 |
| Texto ou razão de fechamento com UTF-8 inválido | 1007 |
| Frame ou mensagem acima do limite | 1009 |

Em uma violação do protocolo, o servidor envia close quando possível e encerra após a escrita. Erros de rede e expiração de uma escrita pendente encerram o socket.

## Limites e backpressure

| Recurso | Política |
| --- | --- |
| Payload de um frame | Até 1 MiB |
| Mensagem reconstituída | Até 1 MiB somando os fragmentos |
| Buffer de saída | Até 1.048.713 bytes, reservando espaço para um echo de 1 MiB e um close |
| Entrada do socket | Leituras de até 8192 bytes, seguidas de parsing antes de nova leitura |
| Abertura HTTP | Usa `Config::idle_timeout` e `Config::request_timeout` existentes |
| Frame incompleto | Prazo absoluto de `Config::request_timeout` contado desde o primeiro byte do frame |
| Ociosidade | `Config::idle_timeout` desde a última leitura da rede |
| Escrita pendente | `Config::write_timeout` sem progresso de escrita |
| Fechamento iniciado pelo servidor | Até o menor valor entre um segundo e `Config::write_timeout` |

O transporte termina a escrita de uma resposta antes de consumir o próximo frame. Quando o cliente para de ler, a produção de ecos também para e a pressão chega ao TCP. A guarda do buffer de saída encerra a conexão caso o limite interno seja excedido. O timeout de escrita libera o recurso de um cliente que permaneça bloqueado.

Os limites de saída descrevem bytes enfileirados. A memória da conexão também inclui o buffer de entrada, a mensagem fragmentada e alocações temporárias limitadas pelo tamanho máximo. Buffers podem conservar sua capacidade após serem esvaziados.

No modo thread, cada sessão ocupa sua thread. No pool, cada sessão ocupa um worker até encerrar. Um pool saturado mantém novas conexões na fila limitada existente e aplica a política atual de rejeição quando ela enche. Epoll e Tokio mantêm sessões persistentes com estado por conexão. O transporte bloqueante usa esperas de até 20 ms por operação para conferir shutdown e deadlines.

Os contadores de requests e responses registram a abertura HTTP. Os contadores de conexões continuam ativos durante a sessão WebSocket.

## Verificar

```bash
cargo test --locked --lib --test websocket
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
RUST_SERV_TEST_MODE=tokio cargo test --locked
```

Os testes exercitam diretamente os cinco caminhos de execução, considerando epoll LT e ET separadamente. A suíte inclui propriedades de round-trip, fragmentação, limites, frames inválidos, clientes lentos, ocupação do pool, EOF e shutdown. Testes em memória simulam leituras fragmentadas, escritas parciais e bloqueio de saída para conferir o limite do buffer sem depender do tamanho dos buffers TCP do sistema.

O experimento abaixo usa Google Chrome e o pacote Python `websocket-client` para controlar DevTools. Ele inicia servidores locais em portas temporárias, usa um perfil temporário do Chrome e interage com os botões da demonstração. Verifica abertura 101, echo de texto e binário e fechamento limpo 1000 nos cinco caminhos.

```bash
cargo build --locked
python3 tests/phase13_browser.py
```

Os resultados locais ficam em `validation/phase13/browser.json`. Esse diretório é ignorado pelo Git. O registro durável das verificações fica em [harness/build-log.md](../harness/build-log.md).

A implementação segue a [RFC 6455](https://www.rfc-editor.org/rfc/rfc6455), especialmente as seções 4 a 8.
