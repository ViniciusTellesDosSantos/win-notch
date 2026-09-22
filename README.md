# win-notch

Overlay estilo "notch" para Windows, inspirado no [codenotch](https://github.com/vinzdg/codenotch): uma pequena pílula fixada numa borda da tela que se expande ao passar o mouse, mostrando informações rápidas.

Nesta primeira versão:

- **Barra de uso do Claude Code** — derivada dos transcripts locais em `~/.claude/projects/**/*.jsonl`, mostrando tokens consumidos na janela móvel de 5h (com um anel indicando quanto dessa janela já passou) e quando ela reinicia. Não é o limite oficial do plano (a Anthropic não publica isso) — é uma estimativa, igual à filosofia do codenotch para fontes não-oficiais.
- **Captura de tela por seleção de região** — clique no botão no painel expandido, arraste um retângulo, ele é copiado direto pra área de transferência. `Esc` cancela.
- **Notch arrastável** — segure e arraste a pílula; ela encaixa na borda (topo/baixo/esquerda/direita) mais próxima de onde você soltar, e a posição fica salva.
- Integração com Google Calendar: **fora de escopo por enquanto** (há um placeholder "Agenda: em breve" no painel).

## Por que Tauri (Rust + HTML/CSS), e não só egui?

A primeira versão deste app era 100% Rust com `egui`/`eframe`. Visualmente ficou aquém do esperado — sem o efeito de vidro fosco (`backdrop-filter: blur()`) que dá o acabamento "premium" do codenotch, e com um bug de reposicionamento ao expandir. Como esse efeito é trivial em CSS e complicado em immediate-mode GUI, o app foi reescrito para **Tauri**: backend em Rust (parsing de uso, captura de tela, janelas, bandeja), interface em HTML/CSS/JS puro servida localmente pelo WebView2 — sem framework JS, sem Node/npm no processo de build (veja abaixo).

## Estrutura

```
├── ui/                       # frontend estático (sem build step — HTML/CSS/JS puro)
│   ├── index.html             # janela do notch (pílula + painel expandido)
│   ├── notch.css / style.css
│   ├── notch.js                # state machine hover, drag+snap de borda, polling de uso
│   ├── selection.html          # overlay fullscreen de seleção de captura
│   └── selection.js
└── src-tauri/
    ├── Cargo.toml
    ├── tauri.conf.json         # janela do notch (transparente, sem decoração, always-on-top)
    ├── capabilities/default.json
    ├── icons/
    └── src/
        ├── main.rs             # bootstrap
        ├── lib.rs               # monta o app Tauri, reposiciona o notch no startup
        ├── commands.rs          # comandos invocáveis do JS (get_usage, capture_region, ...)
        ├── config.rs            # Settings (borda, posição, autostart) persistidos em TOML
        ├── usage/
        │   ├── mod.rs            # thread de polling + snapshot compartilhado
        │   └── claude_code.rs     # parsing dos JSONL e cálculo da janela de 5h
        ├── screenshot.rs        # captura de monitores (xcap) + crop + clipboard (arboard)
        ├── tray.rs              # ícone na bandeja (API nativa do Tauri) e menu
        └── autostart.rs         # toggle "iniciar com o Windows"
```

Toda a geometria (pílula ancorada numa borda, crescendo a partir do centro em vez de "pular" ao expandir, snap de borda ao arrastar) é calculada em pixels **lógicos** tanto no Rust (`config.rs`, usado na posição inicial) quanto no JS (`notch.js`, usado durante hover/drag), convertendo a partir da geometria física do monitor via `scaleFactor` — assim o notch fica do mesmo tamanho relativo em telas com escalas diferentes (100%/125%/150%, muito comuns no Windows).

## Rodando no Windows

Pré-requisitos: [Rust via rustup](https://rustup.rs) + Build Tools do Visual Studio (workload "Desktop development with C++") — ver conversa anterior. WebView2 já vem instalado no Windows 10/11, não precisa instalar nada a mais. Node/npm **não são necessários**.

```powershell
cargo install tauri-cli --version "^2.0" --locked
cd src-tauri
cargo tauri dev      # roda em modo desenvolvimento
cargo tauri build    # gera o instalador (.exe/.msi) em target/release/bundle
```

Ou, sem o `tauri-cli`, só para rodar o binário puro sem empacotar instalador:

```powershell
cd src-tauri
cargo run --release
```

## Cross-compilando para Windows a partir do Linux

O repositório já traz um `.cargo/config.toml` (na raiz) apontando o linker do target `x86_64-pc-windows-gnu` para o `mingw-w64`:

```bash
apt install mingw-w64
rustup target add x86_64-pc-windows-gnu
cd src-tauri
cargo build --release --target x86_64-pc-windows-gnu
```

gera um `win-notch.exe` de verdade em `target/x86_64-pc-windows-gnu/release/`, sem precisar de Windows para compilar (validado neste ambiente). Duas pegadinhas que já foram resolvidas no `Cargo.toml`:

- O crate da lib usa só `crate-type = ["rlib"]`. O template padrão do Tauri usa `["staticlib", "cdylib", "rlib"]` (para suportar mobile), mas o `cdylib` faz o `ld` do mingw-w64 gerar uma tabela de exportação de DLL grande demais (`export ordinal too large`) dada a quantidade de símbolos que a árvore de dependências do Tauri exporta — sem necessidade nenhuma pra um app desktop-only.
- No Linux, o backend do Tauri (WebKitGTK) puxa dependências de sistema (GTK, WebKit2GTK) que só fazem sentido pra compilar *para* Linux — não são necessárias pra cross-compilar pro Windows. Ainda assim, se quiser rodar `cargo test`/`cargo check` no target nativo Linux (útil pra testar a lógica pura mais rápido), instale: `apt install libgtk-3-dev libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libsoup-3.0-dev`.

Isso NÃO gera o instalador (`cargo tauri build` cuida disso e precisa de ferramental específico de Windows tipo NSIS/WiX) — só o `.exe` bruto, suficiente pra testar o app.

## Limitações conhecidas desta v1

- **Multi-monitor**: o cálculo de borda/snap usa o monitor atual da janela; um monitor secundário com posição/escala muito diferente da primária pode se comportar de forma menos precisa. Candidato a melhoria futura.
- **Teto de uso do Claude Code**: como não existe API oficial de quota, o app mostra tokens consumidos + tempo até o reset da janela de 5h (via um anel de progresso baseado no tempo decorrido, não numa porcentagem de limite inventada).
- Sem ícone `.ico` customizado com design real ainda (usa um quadrado sólido gerado programaticamente como placeholder).
- **Nunca testado visualmente numa tela real** — hover/expand, drag entre bordas, o overlay de seleção de captura e o ícone da bandeja foram validados só por compilação (`cargo check`/`cargo build` cross-compilado) e pela leitura cuidadosa da API do Tauri (bundle JS local, não documentação externa, já que este ambiente não tem acesso a ela). Precisa de uma passada manual numa máquina Windows de verdade.

## Verificação feita neste ambiente

```bash
# backend Rust (a partir de src-tauri/)
cargo check --target x86_64-pc-windows-gnu           # valida os caminhos específicos de Windows
cargo build --release --target x86_64-pc-windows-gnu # gera o win-notch.exe de verdade
cargo test                                            # 11 testes unitários (geometria de borda/centro, parsing de uso)
cargo clippy
cargo fmt

# confirmado por manipulação direta: o build falha se ui/ não existir (prova de que o
# frontend é lido e embutido no binário) e volta a passar com ui/ restaurado.
```

Os nomes exatos da API JS do Tauri usados em `ui/notch.js`/`ui/selection.js` (`onMoved`, `startDragging`, `currentMonitor`, `LogicalPosition`, `outerPosition` etc.) foram conferidos direto no bundle `scripts/bundle.global.js` da crate `tauri` baixada localmente — não há acesso a `v2.tauri.app` neste ambiente (bloqueado pela política de rede), então a documentação oficial não pôde ser consultada diretamente.
