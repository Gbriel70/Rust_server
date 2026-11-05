# Fase 13 — WebSocket

## Estado e objetivo

Concluída em 5 de novembro de 2026. A execução foi autorizada pelo usuário em 5 de novembro de 2026. A fase acrescenta uma conexão WebSocket de echo ao servidor, com handshake HTTP e codec de frames compartilhados pelos adaptadores existentes.

A referência de protocolo é a [RFC 6455](https://www.rfc-editor.org/rfc/rfc6455). Ao implementar, consulte as seções de abertura, framing, controle, fragmentação, fechamento e tratamento de erros.

## Escolhas iniciais do plano

- Endpoint `/ws`, com echo de mensagens completas de texto e binário.
- Suporte nos modos thread, pool, epoll e Tokio, com testes específicos dos caminhos de upgrade.
- Limites iniciais de 1 MiB para payload de frame e de mensagem reconstituída. A soma dos fragmentos deve respeitar o limite da mensagem.
- Negociação inicial sem extensões e sem subprotocolos. Uma oferta de extensão pode seguir sem negociação. O servidor omite a seleção de subprotocolo quando a aplicação não o exige.
- A rota de demonstração é pública e sem autenticação. Qualquer uso autenticado em navegador exige definir a política de Origin e credenciais antes de expor esse uso.

Essas escolhas foram propostas na preparação do harness. Mudanças de escopo devem ser registradas antes de comprometer a implementação correspondente.

## Pontos de integração a investigar

- `src/http/parser.rs` e `src/http/borrowed_request.rs` delimitam a requisição que inicia o upgrade.
- `src/connection.rs` e `src/server.rs` controlam a vida útil da conexão nos modos com threads.
- `src/router.rs` e `src/routes.rs` escolhem o atendimento da rota.
- `src/http/response.rs` serializa o status e os headers de abertura.
- `src/event_loop.rs` e `src/tokio_io.rs` mantêm buffers e operações pendentes.
- `tests/support/mod.rs` fornece a base dos testes HTTP compartilhados.

O mecanismo de upgrade precisa transferir o socket, os bytes pendentes e o estado necessário ao atendimento WebSocket. Uma conexão pode receber o pedido HTTP e o começo do primeiro frame na mesma leitura.

## Entregas ordenadas

### 13.1 — Contrato de upgrade e handshake

- Identificar como representar a transição entre HTTP e WebSocket, preservando os contratos dos handlers existentes.
- Validar método GET, HTTP/1.1, Host, tokens de Connection e Upgrade, versão WebSocket e chave conforme a RFC.
- Tratar campos repetidos segundo as regras de cada header. Validar que a chave decodifica para 16 bytes.
- Produzir a resposta 101 com Upgrade, Connection e Sec-WebSocket-Accept corretos. Incluir o vetor de handshake conhecido da RFC no teste.
- Definir respostas HTTP para abertura inválida e versão não suportada, com os headers exigidos pelo protocolo.
- Avaliar primitivas de SHA-1 e Base64, registrar a decisão de dependência e manter o parsing HTTP compartilhado.
- Preservar bytes após o fim da requisição HTTP durante a transferência de controle.

### 13.2 — Codec independente de I/O

- Decodificar incrementalmente frames e produzir o número de bytes consumidos.
- Codificar frames enviados pelo servidor e lidar com escritas parciais nos adaptadores.
- Validar masking do cliente, RSV, opcodes, comprimentos estendidos e representações de comprimento inválidas.
- Validar comprimentos e limites antes de alocar ou acumular payload.
- Reconstituir mensagens fragmentadas, incluindo texto UTF-8 dividido entre frames.
- Processar frames de controle entre fragmentos de dados.
- Validar tamanho e FIN de frames de controle, códigos de fechamento e UTF-8 da razão de fechamento.

### 13.3 — Sessão e adaptadores

- Implementar echo, ping/pong e handshake de fechamento com estado explícito.
- Evitar novas mensagens de dados após a sessão entrar em fechamento e limitar o tempo de espera para concluir o close.
- Integrar thread e pool, depois epoll LT/ET e Tokio, reutilizando o codec e as regras de sessão.
- Aplicar backpressure com fila de saída limitada em bytes. Definir a política para fila cheia e documentar o valor escolhido.
- Definir e testar limites de espera para abertura, ociosidade, escrita e fechamento usando os mecanismos existentes.
- Encerrar sessões durante shutdown com prazo limitado. Tratar EOF, erro de rede e cancelamento sem prender workers ou tarefas indefinidamente.
- Manter `/health` e as demais rotas HTTP utilizáveis durante sessões persistentes.

### 13.4 — Verificação e documentação de uso

- Criar testes do codec em memória e testes de integração pela rota real `/ws`.
- Exercitar um navegador por meio da API WebSocket em uma demonstração mínima ou experimento documentado.
- Registrar os limites, a negociação suportada, o comportamento de fechamento e o consumo de um worker por sessão no modo pool.
- Atualizar a documentação pública relevante e o registro do harness com resultados atuais.

## Critérios de aceitação

- [x] Abertura válida retorna 101 e o accept esperado pelo vetor da RFC.
- [x] Abertura inválida e versão incompatível recebem o tratamento HTTP definido.
- [x] O primeiro frame funciona quando chega junto com o handshake ou dividido em várias leituras.
- [x] Texto e binário retornam corretamente, incluindo mensagens vazias e fragmentadas.
- [x] Ping/pong funciona entre fragmentos e o close respeita os estados e os prazos definidos.
- [x] Frames sem máscara, RSV/opcode inválidos, controle fragmentado e UTF-8 inválido causam o tratamento previsto pelo protocolo.
- [x] Payloads excessivos e comprimentos malformados são rejeitados antes de alocação proporcional ao valor declarado.
- [x] Vários frames na mesma leitura e escritas parciais preservam a ordem das mensagens.
- [x] Um cliente lento respeita o limite da fila de saída e permite progresso de conexões independentes nos modos epoll e Tokio.
- [x] Os modos thread e pool respeitam os limites existentes, com a ocupação de workers por sessão documentada e testada.
- [x] Upgrade, echo, fechamento e shutdown passam nos quatro modos, incluindo epoll LT e ET no Linux.
- [x] A matriz HTTP existente passa com o suporte compartilhado padrão e com Tokio.
- [x] O uso pelo navegador foi exercitado ou sua limitação de ambiente foi registrada como pendência.
- [x] A documentação e o registro refletem os comportamentos e verificações entregues.

## Verificação da fase

Execute a matriz de `AGENTS.md`. Defina os testes focados junto da implementação do codec e dos adaptadores e registre seus comandos reais aqui.

Os testes específicos devem selecionar explicitamente cada modo de execução. `RUST_SERV_TEST_MODE=tokio` cobre os testes que usam o suporte compartilhado atual, e os caminhos de upgrade precisam de cobertura própria em todos os adaptadores.

Os comandos focados da implementação são os seguintes.

```bash
cargo test --locked --lib --test websocket
cargo build --locked
python3 tests/phase13_browser.py
node --check public/websocket.js
```

O experimento de navegador requer Google Chrome e o pacote Python `websocket-client`. O guia de execução, a arquitetura e os limites entregues estão em [docs/websocket.md](../../docs/websocket.md).

Use testes com sockets locais e clientes de teste controláveis para frames inválidos e fragmentação. Defina deadlines nos testes para que uma falha de protocolo produza resultado limitado no tempo. Registre dependências e resultados de qualquer experimento com navegador.

## Evidências para fechar a fase

Registre arquivos alterados, dependências escolhidas, limites finais, resultados dos testes por adaptador, verificações HTTP e procedimento da demonstração. Atualize este estado e o roteiro após satisfazer os critérios. Qualquer critério pendente mantém a fase em andamento.

## Resultado final

Todos os 14 critérios foram atendidos. A suíte padrão e a matriz Tokio passaram com 268 testes cada, zero falhas. O experimento no Chrome passou em thread, pool, Tokio, epoll LT e epoll ET. Os resultados e as decisões estão em [harness/build-log.md](../build-log.md).
