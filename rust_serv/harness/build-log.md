# Registro de progresso

## Estado atual

- Entrega atual concluída — Fase 13, WebSocket, com os 14 critérios atendidos.
- Próxima fase planejada — Fase 14, cliente HTTP, workspace e HTTPS.
- Implementação da Fase 13 — autorizada pelo usuário em 2026-11-05.
- Base local inspecionada — commit `3b0abd7`, branch `main`.
- Sincronização com o GitHub — `origin/main` conferida em 2026-11-05 antes do envio da Fase 13, alinhada à base local `3b0abd7`.

## 2026-11-05 — baseline e criação do harness

### Fontes conferidas

- Roteiro das fases 0 a 17 fornecido pelo usuário nesta conversa.
- `Cargo.toml`, `src/`, `tests/` e `../README.md`.
- `NOTES.md` e registros locais de `validation/phase12/`, disponíveis nesta máquina e ignorados pelo Git.

### Verificações atuais

| Verificação | Resultado observado |
| --- | --- |
| `cargo test --locked` | Código de saída 0, 245 testes aprovados, zero falhas |
| `cargo fmt --check` | Código de saída 0 |
| `cargo clippy --locked --all-targets -- -D warnings` | Código de saída 0 |
| Conferência dos seis documentos do harness | Links locais válidos, blocos de código balanceados e espaços finais conferidos |
| `RUST_SERV_TEST_MODE=tokio cargo test --locked` | Pendente nesta sessão |
| Campanha nova de fuzzing | Pendente nesta sessão |
| Nova execução de Criterion | Pendente nesta sessão |

A primeira tentativa da suíte encontrou restrições para abrir sockets. A execução posterior com acesso às conexões locais concluiu com sucesso. A suíte padrão inclui testes específicos de Tokio, epoll e dos modos com threads. A variável de ambiente acima amplia a execução de testes compartilhados usando Tokio.

O executável de medição de alocações, incluído em `cargo test`, também concluiu. Ele observou 8 para 1 alocações no GET, 7 para 1 no body fixo de 4096 bytes e 8 para 2 no chunked de 4096 bytes, comparando o baseline preservado com o parser atual. Esses valores descrevem o cenário de medição do parser.

O log histórico de fuzzing registra 9.161.985 entradas em 1.801 segundos. Esse resultado pertence à execução anterior registrada na máquina.

### Entrega do harness

Foram criados `AGENTS.md`, `GOALS.md`, `PLANS.md`, `PROMPTS.md`, este registro e `harness/build/phase-13-websocket.md` dentro de `rust_serv/`. Os documentos apontam para os testes existentes e registram critérios para o trabalho futuro.

As escolhas iniciais propostas para a Fase 13 são uma rota `/ws` de echo de texto e binário e integração com os quatro modos de execução. Elas organizam o próximo trabalho e podem ser ajustadas pelo usuário antes ou durante a definição da fase.

A estrutura de instruções foi conferida com a [documentação oficial de AGENTS.md](https://learn.chatgpt.com/docs/agent-configuration/agents-md). A organização do roteiro adapta as convenções do [exemplo oficial de workflows com Codex](https://developers.openai.com/cookbook/examples/codex/iterating-development-workflows-with-codex) ao pacote existente.

### Próximo passo

Quando a Fase 13 for solicitada, começar pelo caminho de upgrade HTTP e pela preservação dos bytes já recebidos. Em seguida, implementar e testar o codec independente de I/O conforme o plano da fase.

## Como registrar uma entrega futura

Acrescente uma entrada datada com a tarefa autorizada, os arquivos alterados, decisões, comandos e resultados. Indique critérios atendidos, falhas de ambiente, trabalho incompleto e o próximo passo concreto. Mantenha a seção de estado atual coerente com a entrada mais recente.

## 2026-11-05 — implementação da Fase 13

### Autorização e entregas

O usuário solicitou a execução da Fase 13 seguindo o harness. Foram mantidas as escolhas de `/ws`, echo de texto e binário e suporte nos quatro modos, incluindo epoll LT e ET.

- `src/websocket/` contém handshake, codec incremental, sessão independente de I/O e estado de transporte compartilhado.
- `Body::WebSocket` e o status 101 sinalizam a transferência de controle. Os adaptadores preservam os bytes recebidos após a requisição HTTP.
- Os handlers HTTP conservam o retorno `Response`. As rotas e os transportes existentes receberam a integração de upgrade.
- Foram acrescentados 23 testes, considerando 20 testes em `tests/websocket.rs` e três testes internos do transporte. Os testes parametrizados percorrem os modos explicitamente.
- `public/websocket.html` e `public/websocket.js` fornecem a demonstração com a API nativa do navegador, ligada pela navegação da página inicial.
- `docs/websocket.md` documenta arquitetura, limites e execução. O README principal foi atualizado para descrever o suporte entregue.
- `tests/phase13_browser.py` reproduz a verificação dos botões da demonstração em Chrome com perfil temporário e servidores em portas temporárias.

### Decisões de implementação

- As dependências diretas novas são `sha1` e `base64`. O lockfile resolveu SHA-1 0.10.7 e Base64 0.22.1. Essas bibliotecas fornecem as primitivas exigidas pelo handshake.
- Frame e mensagem têm limite de 1 MiB. A saída tem limite de 1.048.713 bytes, incluindo espaço reservado para close. As leituras param enquanto há saída pendente, aplicando backpressure.
- Fechamento iniciado pelo servidor tem prazo máximo de um segundo, reduzido quando o timeout de escrita configurado é menor. Shutdown e expiração de leitura iniciam close 1001. O transporte bloqueante usa esperas de até 20 ms para observar prazos e shutdown.
- Cada sessão ocupa um worker nos modos bloqueantes. O teste de saturação do pool demonstra a liberação do worker após close. Epoll e Tokio mantêm `/health` responsivo durante backpressure de outro cliente.
- A rota de demonstração mantém o escopo público previsto no plano. Extensões e subprotocolos oferecidos seguem sem negociação.

### Evidências observadas

| Verificação | Resultado |
| --- | --- |
| `cargo test --locked` na versão final | Código de saída 0, 268 testes aprovados, zero falhas |
| `RUST_SERV_TEST_MODE=tokio cargo test --locked` na versão final | Código de saída 0, 268 testes aprovados, zero falhas |
| `cargo fmt --check` | Código de saída 0 |
| `cargo clippy --locked --all-targets -- -D warnings` | Código de saída 0 |
| `cargo build --locked` | Código de saída 0 |
| `node --check public/websocket.js` | Código de saída 0 |
| `python3 tests/phase13_browser.py` | Código de saída 0, cinco caminhos aprovados no Chrome 156 |
| Links locais e espaços finais dos documentos e arquivos novos | Conferidos |

O Chrome verificou resposta 101, echo de `Olá €`, echo binário de `0, 1, 2, 255` e close limpo 1000 em thread, pool, Tokio, epoll LT e epoll ET. O experimento encerrou os servidores e o perfil temporário. Os resultados locais estão em `validation/phase13/browser.json`.

Os testes de integração cobrem também upgrade após HTTP keep-alive, handshake e primeiro frame juntos, fragmentação de UTF-8, ping intercalado, comprimentos não mínimos, frames inválidos, limites, EOF, deadlines e shutdown com peer sem resposta. Os testes internos simulam escritas parciais e bloqueio para conferir a ordem dos frames e a saída limitada.

O parser HTTP permaneceu com a implementação anterior. Nesta fase, a verificação adicional do codec usa propriedades com Proptest e testes determinísticos. A campanha histórica de fuzzing HTTP e os benchmarks de Criterion permanecem associados à Fase 12.

### Próximo passo da entrega

A Fase 13 está concluída. O próximo trabalho é detalhar o plano da Fase 14 a partir da base entregue, quando solicitado pelo usuário. Os testes e o experimento de navegador foram encerrados.
