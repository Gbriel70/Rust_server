# Plano de continuidade

## Ponto de partida

O código local foi inspecionado em 5 de novembro de 2026, na base Git `3b0abd7`. Há implementações correspondentes às fases 0 a 12 e uma suíte padrão com 245 testes aprovados nessa inspeção. Antes do envio da Fase 13, `origin/main` foi consultada e estava alinhada a essa base.

O marco abaixo descreve a implementação existente. A reprodução de cada experimento histórico, incluindo strace, tcpdump, benchmarks e fuzzing, tem verificação própria.

## Roteiro

| Fase | Capacidade | Situação |
| --- | --- | --- |
| 0 | Compilação e estrutura do projeto | Implementação existente |
| 1 | TCP com accept, read e write | Implementação existente |
| 2 | Parser HTTP incremental e independente de I/O | Implementação existente |
| 3 | Response, Status, erros e testes em porta 0 | Implementação existente |
| 4 | Router, 404, 405 e HEAD | Implementação existente |
| 5 | Keep-alive, timeouts e limites | Implementação existente |
| 6 | Arquivos estáticos e proteção de caminhos | Implementação existente |
| 7 | Threads, pool, compartilhamento e shutdown | Implementação existente |
| 8 | epoll e estado por conexão | Implementação existente |
| 9 | Tokio e compatibilidade HTTP | Implementação existente |
| 10 | Streaming, chunked e backpressure | Implementação existente |
| 11 | Range, 206, 416 e offsets de arquivos | Implementação existente |
| 12 | Parsing emprestado, propriedades, fuzzing e medição | Implementação existente |
| 13 | WebSocket | Concluída em 2026-11-05 |
| 14 | Cliente HTTP, workspace e HTTPS com rustls | Próxima fase, planejada |
| 15 | Proxy e toolkit | Planejada |
| 16a | Bencode | Planejada |
| 16b | info_hash e magnet | Planejada |
| 16c | Tracker | Planejada |
| 16d | Peer wire | Planejada |
| 16e | Download completo | Planejada |
| 16f | Streaming por Range | Planejada |
| 16g | DHT | Planejada |
| 17 | Estudo de stremio-core e mini-runtime | Planejada |

A etapa opcional 9b deve ser conferida separadamente se for retomada. A existência dos testes de timeout manual é um ponto de entrada para essa conferência.

## Execução das próximas fases

A Fase 13 tem plano próprio em [harness/build/phase-13-websocket.md](harness/build/phase-13-websocket.md). As fases seguintes serão detalhadas ao se tornarem a tarefa atual, usando o código resultante das entregas anteriores.

| Fase | Resultado a detalhar | Verificação central |
| --- | --- | --- |
| 14 | Cliente reutilizável, separação de crates e transporte TLS | Respostas fragmentadas, framing, limites, falhas de conexão e certificados em ambiente local |
| 15 | Proxy exercitado com cliente e servidor locais | Encaminhamento, headers hop-by-hop, streaming, backpressure e falhas do upstream |
| 16a | Encoder e decoder de bencode | Vetores válidos e inválidos, profundidade e limites de tamanho |
| 16b | Identidade do torrent e parsing de magnet | Hash dos bytes originais de info, encoding e parâmetros repetidos |
| 16c | Comunicação com tracker | Fixture local, resposta inválida, timeout e tratamento de peers |
| 16d | Codec e sessão peer wire | Fragmentação, limites, handshake e transições da sessão |
| 16e | Download verificado | Peers locais, hash das peças, escrita consistente e retomada definida |
| 16f | Arquivo em aquisição servido por Range | Solicitação de peças ausentes, cancelamento, concorrência e uso de memória |
| 16g | Descoberta DHT | Rede local de nós de teste, validação de mensagens e limites de consulta |
| 17 | Integração ou mini-runtime com contrato definido | Cenário demonstrável escolhido após o estudo da versão de stremio-core utilizada |

## Ciclo de entrega

1. Identificar a tarefa autorizada, consultar as instruções e conferir o estado local.
2. Ler o plano da fase e localizar os contratos afetados no código e nos testes.
3. Registrar decisões relevantes e implementar em passos verificáveis.
4. Executar testes focados e a matriz de verificação aplicável.
5. Conferir o diff e atualizar `harness/build-log.md` com evidências e pendências.
6. Marcar a fase concluída somente quando os critérios estiverem satisfeitos.

Uma retomada parte do registro e do estado real dos arquivos. Trabalho parcialmente executado exige conferir processos, saídas e alterações já existentes antes de repetir operações.
