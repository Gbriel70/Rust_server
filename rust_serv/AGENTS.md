# Orientações para trabalhar neste projeto

## Objetivo e contexto

Este pacote é um servidor HTTP em Rust para aprender protocolos, memória e concorrência por meio de implementações próprias. O desenvolvimento continuará com assistência de IA. Comunique-se com o usuário em português e explique brevemente as decisões que ajudam a entender o código entregue.

O repositório Git está um nível acima deste diretório. Os caminhos e comandos deste harness são relativos a `rust_serv/`, onde fica o `Cargo.toml`.

## Onde encontrar o contexto

- `GOALS.md` define os resultados esperados e as restrições do projeto.
- `PLANS.md` contém o roteiro e a ordem de execução.
- `harness/build-log.md` registra progresso observado, verificações e pendências.
- `harness/build/phase-13-websocket.md` detalha a próxima fase.
- `PROMPTS.md` contém instruções reutilizáveis para iniciar e retomar trabalho.
- `../README.md` descreve a aplicação existente e seus comandos.
- `NOTES.md`, quando disponível, contém explicações e experimentos históricos. Ele é ignorado pelo Git. O harness versionado deve ser suficiente para continuar em outra máquina.

Leia os documentos relevantes à tarefa. Consulte as implementações e os testes afetados antes de modificar comportamento. Confirme no código as afirmações históricas que influenciam a mudança.

## Forma de trabalhar

- Execute a tarefa ou fase autorizada até cumprir seus critérios. Resolva escolhas locais e reversíveis seguindo as convenções existentes.
- Um pedido de preparação do harness autoriza a estrutura de trabalho. A implementação de uma fase começa quando o usuário a solicitar.
- Trabalhe na fase solicitada e mantenha as seguintes no roteiro. Atualize o plano quando surgir uma dependência real.
- Preserve alterações locais existentes. Faça mudanças pequenas e relacionadas ao objetivo.
- Pergunte quando faltar uma decisão de produto que altere materialmente o escopo. Registre as suposições reversíveis adotadas.
- Atualize o registro de progresso ao concluir uma entrega ou interromper trabalho incompleto. Uma nova conversa deve conseguir identificar o próximo passo.

## Arquitetura a preservar

- O protocolo HTTP, o roteamento e os arquivos estáticos são compartilhados pelos modos thread, pool, epoll e Tokio.
- O parser deve continuar independente de sockets e do runtime. Leituras TCP podem conter mensagens parciais ou várias mensagens juntas.
- A API emprestada, os planos com índices e `RequestFrame` permitem controlar ownership e lifetimes. Preserve os contratos das APIs públicas usadas pelos testes.
- Reaproveite os controles existentes de limites, deadlines, escrita parcial, backpressure e shutdown.
- epoll é específico de Linux. Delimite o código e os testes de plataforma com a configuração existente.
- Considere o código e os testes como fonte do comportamento atual. Registros de medições descrevem a execução em que foram produzidos.

## Convenções de implementação

- Use Rust legível, nomes descritivos e abstrações justificadas por uso real.
- Implemente o protocolo da fase dentro da arquitetura existente. Avalie dependências pequenas para primitivas auxiliares e documente a decisão.
- Mantenha os caminhos existentes de HTTP funcionando ao adicionar protocolos e adaptadores.
- Acrescente testes para o comportamento introduzido e para regressões concretas. Exercite entradas fragmentadas, inválidas e os limites relevantes.
- Todo `unsafe` novo exige justificativa localizada e invariantes explícitas. Prefira recursos seguros quando atenderem ao mesmo contrato.

## Verificação

Execute os testes focados durante o desenvolvimento. Para fechar uma fase que altera código Rust, execute os comandos abaixo a partir deste diretório.

```bash
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
RUST_SERV_TEST_MODE=tokio cargo test --locked
```

A variável seleciona Tokio nos testes que usam o suporte compartilhado. Os testes específicos de cada adaptador também precisam passar. Use as verificações adicionais da fase para cobrir os modos e caminhos novos.

Os testes de integração precisam abrir conexões TCP locais. Se uma restrição do ambiente impedir a execução, registre o comando e a limitação e use o mecanismo de permissão disponível para tentar completar a verificação. Distinga falha de ambiente de falha do programa.

Benchmarks e fuzzing têm comandos e dependências próprios. Execute-os quando a alteração afetar o parser, seus limites ou desempenho, ou quando o plano da fase exigir. Registre o orçamento de execução e o resultado observado.

## Critério de conclusão

Uma fase fica concluída quando seus critérios de aceitação foram atendidos, os testes relevantes passaram e o registro de progresso contém evidências atuais. Relate comandos não executados, falhas e limitações restantes. Informe os arquivos alterados e as principais decisões na entrega.

Em mudanças exclusivas de documentação, confira os caminhos, os links, a consistência entre os arquivos e o diff. A matriz completa de testes se aplica às alterações de código.

O mecanismo de descoberta destas instruções está descrito na [documentação oficial de AGENTS.md](https://learn.chatgpt.com/docs/agent-configuration/agents-md).
