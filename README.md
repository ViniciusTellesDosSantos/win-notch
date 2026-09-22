# win-notch

Overlay estilo "notch" para Windows, inspirado no [codenotch](https://github.com/vinzdg/codenotch): uma pequena pílula fixada numa borda da tela que se expande ao passar o mouse, mostrando informações rápidas.

Nesta primeira versão:

- **Barra de uso do Claude Code** — derivada dos transcripts locais em `~/.claude/projects/**/*.jsonl`, mostrando tokens consumidos na janela móvel de 5h e quando ela reinicia. Não é o limite oficial do plano (a Anthropic não publica isso) — é uma estimativa, igual à filosofia do codenotch para fontes não-oficiais.
- **Captura de tela por seleção de região** — clique no botão no painel expandido, arraste um retângulo, ele é copiado direto pra área de transferência. `Esc` cancela.
- **Notch arrastável** — segure e arraste a pílula; ela encaixa na borda (topo/baixo/esquerda/direita) mais próxima de onde você soltar.
- Integração com Google Calendar: **fora de escopo por enquanto** (há um placeholder "Agenda: em breve" no painel).

## Rodando no Windows

```powershell
cargo run --release
```

O binário fica em `target/release/win-notch.exe`. Não há instalador nesta versão — é só rodar o `.exe` (ou colocar um atalho na pasta de Inicialização do Windows, já que o toggle "Iniciar com o Windows" do menu da bandeja cuida disso automaticamente via registro).

## Estrutura

```
src/
├── main.rs           bootstrap do eframe + janela inicial
├── app.rs             App principal: liga notch, uso, captura de tela e bandeja
├── config.rs          Settings (borda, posição, autostart) persistidos em TOML
├── notch/
│   ├── mod.rs          state machine collapsed/expanded + drag
│   ├── edge.rs          geometria: posição na borda, snap, clamp
│   └── shape.rs         desenho da pílula/painel
├── usage/
│   ├── mod.rs           thread de polling + snapshot compartilhado
│   └── claude_code.rs    parsing dos JSONL e cálculo da janela de 5h
├── screenshot/         captura de monitores (xcap) + overlay de seleção + clipboard
├── tray.rs             ícone na bandeja e menu
└── autostart.rs        toggle "iniciar com o Windows"
```

## Limitações conhecidas desta v1

- **Multi-monitor**: a posição do monitor é assumida como (0,0) — funciona bem no caso comum de monitor único; numa segunda tela que não comece na origem, o snap de borda pode ficar impreciso. Candidato a melhoria futura.
- **Teto de uso do Claude Code**: como não existe API oficial de quota, o app mostra tokens consumidos + tempo até o reset da janela de 5h, sem inventar uma porcentagem de limite.
- Sem ícone `.ico` customizado ainda (usa um quadrado sólido como placeholder na bandeja).
- Só testado por `cargo check`/`cargo test`/`cargo clippy` neste ambiente Linux (nativo + cross-check para `x86_64-pc-windows-gnu`, sem linkedição real). **A validação visual (pílula, hover, drag entre bordas, overlay de captura, bandeja) ainda precisa ser feita rodando numa máquina Windows de verdade** — não há Windows disponível neste ambiente de desenvolvimento.

## Verificação feita neste ambiente

```bash
cargo check                                  # target nativo (Linux) — módulos multiplataforma
cargo check --target x86_64-pc-windows-gnu   # valida também os caminhos específicos de Windows
cargo test                                   # 18 testes unitários (geometria da borda, state machine, parsing de uso)
cargo clippy --all-targets
cargo fmt
```
