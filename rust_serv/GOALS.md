# Objetivos do projeto

## Propósito

Construir progressivamente um servidor e ferramentas de rede em Rust, mantendo visíveis as decisões sobre bytes, protocolos, ownership, concorrência e uso de recursos. A IA participa da implementação, dos testes e da documentação. As entregas devem continuar compreensíveis para o usuário que está aprendendo Rust.

## Resultados esperados

- Continuar a base HTTP existente a partir da Fase 13 do roteiro fornecido pelo usuário.
- Acrescentar WebSocket, cliente HTTP com HTTPS e organização em workspace conforme as respectivas fases.
- Evoluir para proxy e toolkit de rede.
- Implementar progressivamente as peças de BitTorrent e a integração com streaming por Range.
- Chegar ao estudo de `stremio-core` e à definição de um mini-runtime na Fase 17.

A relação entre BitTorrent, streaming e o mini-runtime vem do roteiro fornecido. Os requisitos específicos dessas entregas serão detalhados quando cada fase se aproximar.

## Restrições e qualidade

- Preservar os quatro modelos atuais de atendimento e a suíte de comportamento HTTP existente.
- Manter o parsing e as regras de protocolo testáveis em memória, separados do transporte.
- Limitar memória, tamanho de mensagens, conexões e tempo de espera nos caminhos em que esses recursos são consumidos.
- Tratar escrita parcial, desconexão, cancelamento e shutdown como partes do contrato.
- Manter os componentes novos verificáveis por testes locais e determinísticos. Experimentos externos precisam ter dependências e resultados identificados.
- Adotar dependências de forma deliberada. HTTPS usará uma implementação de TLS mantida, conforme a Fase 14 prevê com rustls.
- Conservar documentação suficiente no Git para retomar o trabalho sem depender de chats antigos ou de arquivos pessoais ignorados.

## Evidência de sucesso

Cada fase entrega uma capacidade exercitável pela entrada normal da aplicação, testes do protocolo e da integração, instruções de execução e registro do que foi verificado.

O roteiro organiza o trabalho futuro. A autorização para executar uma fase vem da tarefa atual do usuário. A preparação deste harness é uma entrega própria, anterior à implementação de WebSocket.
