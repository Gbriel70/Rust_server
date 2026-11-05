# Prompts para trabalhar com a IA

Os exemplos abaixo podem ser usados no Codex com este pacote aberto. Substitua o número da fase quando necessário.

## Iniciar a próxima fase

```text
Leia AGENTS.md, GOALS.md, PLANS.md, harness/build-log.md e o plano
harness/build/phase-13-websocket.md.

Execute a Fase 13 até cumprir os critérios de aceitação. Confira primeiro
o estado do projeto e preserve as alterações existentes. Implemente em
passos pequenos e exercite o recurso pelas entradas normais do servidor.

Use as escolhas iniciais documentadas no plano. Resolva escolhas locais
com base no código existente e registre decisões relevantes. Se faltar
uma decisão que altere materialmente o escopo, faça uma pergunta objetiva
e avance no trabalho independente dessa resposta.

Execute as verificações aplicáveis, confira o diff e atualize o plano e
o registro com os resultados observados. Explique brevemente as decisões
de protocolo e Rust que ajudam a entender a entrega. Encerre com o estado
da fase, os testes executados e as pendências reais.
```

## Retomar uma fase interrompida

```text
Leia AGENTS.md, PLANS.md, harness/build-log.md e o plano da fase em andamento.
Confira o código, as alterações locais e os processos ou testes deixados
pela execução anterior. Identifique o último passo realmente concluído.

Continue a fase autorizada até seus critérios serem atendidos. Preserve o
trabalho existente e atualize as evidências e o próximo passo no registro.
```

## Detalhar uma fase futura

```text
Leia AGENTS.md, GOALS.md, PLANS.md e harness/build-log.md. Detalhe a Fase 14
com base no código atual e no resultado das fases anteriores.

Crie o plano da fase com objetivo, decisões, entregas ordenadas, critérios
de aceitação e comandos de verificação. Identifique suposições e decisões
que dependem do usuário. Esta tarefa termina na preparação do plano.
```

## Consultar o progresso

```text
Leia PLANS.md e harness/build-log.md. Confira as evidências e o estado local.
Resuma a fase atual, as entregas concluídas, as verificações pendentes e
o próximo passo executável. Esta tarefa é uma consulta de estado.
```
