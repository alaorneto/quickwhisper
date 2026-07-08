# QuickWhisper — Plano de Arquitetura e Implementação

> **Status:** Fases 0–3 implementadas (núcleo + histórico + extensão overlay/auto-paste). Fase 4 pendente.
> **Data:** 2026-07-06
> **Alvo:** Rocky Linux 10.x · GNOME 49 · Wayland · Rust

## 0. Descobertas da Fase 0 (2026-07-06, máquina real)

- **Spike A (hotkey): o portal GlobalShortcuts NÃO existe no Rocky 10.** O sistema roda GNOME
  Shell 49.4, mas o `xdg-desktop-portal-gnome` 47.3 empacotado não declara a interface
  (verificado via introspecção do bus e do arquivo `gnome.portal`). **Decisão: evdev virou o
  caminho primário** — leitura direta de `/dev/input` (requer usuário no grupo `input`), com
  press/release exatos; auto-repeat do kernel (value=2) é filtrado. O código isola isso em
  `src/hotkey.rs` para permitir um backend de portal no futuro.
- Sessão confirmada: Wayland, PipeWire, `wl-clipboard` presente; Mutter ≥46 suporta
  ext-data-control (clipboard via `wl-clipboard-rs` ok).
- rubato 3.x mudou a API (audioadapter); fixado **rubato 0.16** (API clássica, suficiente
  para resample offline).
- Fase 1 ficou **sem tokio** de propósito: máquina de estados síncrona + threads (evdev,
  captura, clipboard, notificações). Async entra na Fase 3 se o zbus pedir.
- Spike B (extensão: overlay + VirtualInputDevice) adiado para o início da Fase 3.

**Resultados dos testes ponta a ponta (mesma data):**

- **Spike A confirmado em uso real:** press/release do F12 via evdev perfeitos, auto-repeat
  filtrado, 2 teclados monitorados.
- **Spike C (CPU local, sem GPU):** `base` = 0.28x tempo real; `small` = 0.99x com detecção de
  idioma, **0.31x com idioma fixo**. Ditado real de 18 s em pt-BR transcrito impecável em 5.7 s.
- **Auto-detect falhou em frase curta** ("teste teste teste" → detectou polonês). Decisão do
  usuário: **`language = "pt"` virou o default**, alterável via `quickwhisper config set
  language <código|auto>` (subcomando `config get/set/show` implementado na Fase 1).
- **Clipboard:** o Mutter do Rocky 10 NÃO expõe ext/wlr-data-control a clientes comuns —
  `wl-clipboard-rs` falha e o daemon usa o binário `wl-copy` (que cria uma surface oculta).
  O fallback é automático e sticky. Na Fase 3, preferir setar o clipboard pela extensão
  (`St.Clipboard`), que roda dentro do Shell.
- Logs do whisper.cpp roteados para `tracing` (feature `tracing_backend` do whisper-rs).

**Descobertas da Fase 2 (2026-07-06):**

- **Ditados emendados eram perdidos:** com a máquina de estados bloqueando no whisper, um
  F12 pressionado durante o processamento era descartado em silêncio (aconteceu em teste
  real). Correção: **worker de transcrição com fila** (thread + mpsc) — gravação nova pode
  começar imediatamente enquanto a anterior transcreve. O `drain_stale_keys` foi removido.
- **wl-copy pode travar** disputando a seleção com um wl-copy antigo ainda servindo o
  clipboard (observado uma vez; não determinístico). O `child.wait()` sem limite congelava o
  worker para sempre. Correção: espera com timeout de 3s + kill; perder uma cópia é melhor
  que perder o daemon (o texto sobrevive na notificação e no histórico).
- Histórico SQLite (rusqlite bundled): timestamps UTC no banco, conversão para hora local
  feita pelo próprio SQLite (`strftime(..., 'localtime')`) — zero dependência de crate de data.
- Atenção: `edition` do Cargo.toml deve permanecer `2021` (uma edição "2026" não existe no
  Rust estável e quebra todos os comandos cargo).

**Descobertas da Fase 3 (2026-07-07):**

- **Spike B validado no GNOME 49** via shell aninhado (`dbus-run-session -- gnome-shell
  --nested --wayland`): extensão ACTIVE, ciclo completo de sinais simulado por `busctl emit`
  sem exceções JS — incluindo `addTopChrome` (pílula) e `Clutter.VirtualInputDevice` (Ctrl+V).
  O shell aninhado é o caminho de teste sem relogar; a sessão real exige logout/login.
- D-Bus server no daemon: zbus 5 **blocking** (`zbus::blocking::connection::Builder` +
  `#[zbus::interface]`), sem runtime async; sinais emitidos com `zbus::block_on`. Interface
  conforme §5; presença da extensão detectada por `NameHasOwner` a cada ditado.
- Notificação só é exibida quando a extensão está ausente; com overlay presente, o feedback é
  visual + auto-paste. `Finished` só é emitido após clipboard OK (evita colar conteúdo velho).
- Ordem com ditados encadeados: a extensão só esconde a pílula em `Finished/Failed/Cancelled`
  se estiver em `processing` — se já houver nova gravação em curso, a pílula permanece.
- Extensão instalada por symlink do repo (dev); `enabled-extensions` já persistido no dconf.

## 1. O que é

Utilitário push-to-talk para Linux:

1. Usuário **segura F12** (configurável) → começa a gravar o microfone.
2. Um **overlay em pílula** aparece na parte inferior da tela com animação de onda sonora (gradiente roxo→laranja), indicando gravação.
3. Usuário **solta a tecla** → a onda vira uma animação de processamento enquanto o Whisper transcreve localmente.
4. Ao terminar: o texto é **colado no campo de texto ativo**, **copiado para a área de transferência**, salvo no **histórico**, e o overlay desaparece.
5. Uma **CLI** lista o histórico com timestamps e permite copiar ou apagar entradas.

Decisões já tomadas com o usuário:

| Decisão | Escolha |
|---|---|
| Engine de transcrição | **Local, whisper.cpp** (via `whisper-rs`) — offline e privado |
| Idioma | **Auto-detecção** pelo Whisper |
| Tecla padrão | **F12**, segurando (push-to-talk); configurável |
| Feedback visual | Overlay pílula, bottom-center, onda sonora com gradiente roxo→laranja |
| Histórico | CLI: listar (com timestamp), copiar, apagar |

## 2. Restrições do Wayland que moldam a arquitetura

Estas três restrições são o motivo da arquitetura em dois processos (daemon + extensão do Shell):

1. **Atalho global:** Wayland proíbe key-grab global por clientes. O caminho padrão é o portal
   `org.freedesktop.portal.GlobalShortcuts` (disponível no GNOME 45+; Rocky 10 = GNOME 47 ✅),
   que emite sinais `Activated`/`Deactivated` — exatamente o par press/release que o push-to-talk precisa.
2. **Overlay posicionado:** Mutter **não implementa** wlr-layer-shell. Uma janela GTK não pode se
   posicionar em "bottom-center da tela" no Wayland. O único caminho nativo para um OSD posicionado
   (como o OSD de volume do GNOME) é uma **extensão do GNOME Shell** (GJS/Clutter), que roda dentro
   do compositor e desenha onde quiser.
3. **Colar no app ativo:** `wtype` não funciona no Mutter. Opções reais:
   - **Extensão do Shell** com `Clutter.VirtualInputDevice` sintetizando `Ctrl+V` (roda no compositor — funciona);
   - Portal `RemoteDesktop` + libei (GNOME 46+, crate `reis`/`ashpd`) — funciona sem extensão, mas pede permissão via diálogo;
   - `ydotool` (requer daemon + permissão em `/dev/uinput`) — última opção.

Como a extensão já é necessária para o overlay (item 2), ela também resolve o item 3 de graça.

## 3. Arquitetura

```
┌──────────────────────────────┐        D-Bus (bus de sessão)        ┌──────────────────────────────┐
│  quickwhisper (daemon Rust)  │ ─────────────────────────────────▶  │  Extensão GNOME Shell (GJS)  │
│                              │  sinais: RecordingStarted,          │                              │
│  • Portal GlobalShortcuts    │          AudioLevel(rms),           │  • Overlay pílula (Clutter)  │
│  • Captura de áudio (cpal)   │          Processing,                │  • Animação onda/processo    │
│  • whisper-rs (transcrição)  │          Finished(text) / Error     │  • Ctrl+V via VirtualInput   │
│  • Histórico (SQLite)        │ ◀─────────────────────────────────  │  • St.Clipboard (fallback)   │
│  • Clipboard (wl-clipboard)  │  método: PasteDone / erros          │                              │
│  • Config (TOML)             │                                     └──────────────────────────────┘
└──────────────────────────────┘
        ▲
        │ mesmo binário, subcomandos
┌──────────────────────────────┐
│  CLI: history list/copy/     │
│  delete · model download ·   │
│  config get/set              │
└──────────────────────────────┘
```

### 3.1 Daemon (`quickwhisper daemon`)

Processo residente (systemd user service + autostart). Responsabilidades:

- Registrar atalho no portal GlobalShortcuts (trigger preferido: F12) via `ashpd`.
- Capturar áudio no press, parar no release.
- Transcrever com `whisper-rs` em thread bloqueante (`tokio::task::spawn_blocking`).
- Publicar serviço D-Bus `io.github.alaor.QuickWhisper` com sinais de estado para a extensão.
- Persistir histórico, setar clipboard, disparar notificação em erro.

**Máquina de estados:**

```
                 F12 press                    F12 release
   ┌────────┐  (Activated)   ┌───────────┐  (Deactivated)  ┌────────────┐
   │  Idle  │ ─────────────▶ │ Recording │ ──────────────▶ │ Processing │
   └────────┘                └───────────┘                 └────────────┘
       ▲                          │  grava PCM,                  │ whisper
       │                          │  emite AudioLevel ~15Hz      │ (blocking thread)
       │      overlay some        ▼                              ▼
       └──────────────────── [Finishing: salva histórico → clipboard → paste → sinal Finished]
```

Regras de borda (ver §7): gravação < 1 s é descartada; press durante `Processing` é ignorado
com feedback visual; transcrição vazia não cola nada nem entra no histórico.

### 3.2 Pipeline de áudio

- **Captura:** `cpal` (backend ALSA→PipeWire no Rocky). Formato de captura: nativo do dispositivo.
- **Conversão:** downmix para mono + resample para **16 kHz f32** (requisito do whisper.cpp) com `rubato`.
- **Buffer:** acumulado em memória (`Vec<f32>`); ditado típico < 60 s ⇒ < 4 MB, sem necessidade de arquivo temporário.
- **Nível RMS:** calculado por bloco (~64 ms) e emitido como sinal D-Bus `AudioLevel(f64)` para alimentar a animação da onda (a onda reage à voz real, não é decorativa).
- **Limite de segurança:** gravação máxima configurável (default 120 s) para caso de tecla presa.

### 3.3 Transcrição

- `whisper-rs` (bindings do whisper.cpp), idioma `auto`.
- Modelos ggml em `~/.local/share/quickwhisper/models/`; download via
  `quickwhisper model download <base|small|medium>` (Hugging Face, repositório `ggerganov/whisper.cpp`).
- Default sugerido: **small** (bom equilíbrio precisão × latência em CPU; validar na Fase 0 com hardware real).
- Contexto do Whisper carregado **uma vez** no boot do daemon e mantido em memória (evita ~1–3 s de load por ditado). Flag de config `unload_after_idle_min` para quem quiser liberar RAM.

### 3.4 Extensão GNOME Shell (overlay + paste)

Extensão GJS mínima, instalada em `~/.local/share/gnome-shell/extensions/quickwhisper@alaorneto.github.io`.

- Conecta aos sinais D-Bus do daemon.
- **Overlay:** `St.Widget` adicionado via `Main.layoutManager.addTopChrome()` — mesma técnica dos OSDs nativos. Posição: bottom-center, **monitor onde está o ponteiro** (config futura: todos os monitores).
- **Paste:** ao receber `Finished(text)`, seta o clipboard via `St.Clipboard` e sintetiza `Ctrl+V` com `Clutter.VirtualInputDevice` (mesma técnica de extensões de teclado virtual).
- Não recebe foco, não intercepta input, `reactive: false`.

**Design do overlay (pílula):**

```
        estado Recording                       estado Processing
   ╭──────────────────────────╮           ╭──────────────────────────╮
   │  ● ▂▄▇█▆▃▂▁▂▄▆█▇▄▂ 🎙    │    ──▶    │      ◌ shimmer/pulse     │
   ╰──────────────────────────╯           ╰──────────────────────────╯
     barras reagem ao RMS real              gradiente varre a pílula
```

- **Forma:** pílula (border-radius total), fundo escuro translúcido (consistente com OSDs do GNOME), sombra suave.
- **Onda:** ~24 barras verticais; altura = interpolação suavizada do `AudioLevel`; preenchimento com **gradiente linear roxo→laranja** (referência: `#7c3aed → #f97316`), animado via `Clutter` a 60 fps com easing.
- **Processing:** as barras colapsam para uma linha e um brilho (shimmer) do mesmo gradiente percorre a pílula em loop.
- **Saída:** fade-out + leve slide-down (~200 ms) ao concluir.
- **Acessibilidade (exigência da HIG):** respeitar `org.gnome.desktop.interface enable-animations` — com animações desligadas, mostrar pílula estática com ícone de microfone (gravando) e `AdwSpinner`-like estático (processando); funcionar em alto contraste (borda visível).

**Degradação sem extensão:** se a extensão não estiver instalada/ativa, o daemon detecta a ausência do nome D-Bus dela e degrada para: notificação GNOME "Gravando… / Transcrito (copiado)" + apenas clipboard (sem auto-paste). O app continua útil.

### 3.5 Histórico + CLI

- **Armazenamento:** SQLite (`rusqlite`) em `~/.local/share/quickwhisper/history.db`.

```sql
CREATE TABLE transcriptions (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  created_at  TEXT NOT NULL,        -- ISO 8601 UTC; exibido em hora local
  text        TEXT NOT NULL,
  duration_ms INTEGER NOT NULL,     -- duração do áudio
  lang        TEXT,                 -- idioma detectado pelo whisper
  model       TEXT NOT NULL         -- ex.: "small"
);
```

- **CLI** (mesmo binário, `clap` com subcomandos):

```
quickwhisper history list [--limit N] [--json]   # tabela: id · data/hora local · idioma · trecho do texto
quickwhisper history show <id>                   # texto completo
quickwhisper history copy <id>                   # copia para o clipboard (wl-clipboard-rs)
quickwhisper history delete <id>...              # apaga entradas
quickwhisper history clear [--yes]               # apaga tudo (pede confirmação)
quickwhisper model download <base|small|medium>  # baixa modelo ggml
quickwhisper config get|set <chave> [valor]      # lê/edita config.toml
quickwhisper daemon                              # roda o daemon (usado pelo systemd)
quickwhisper status                              # daemon ativo? modelo carregado? extensão presente?
```

- CLI lê o SQLite diretamente (sem depender do daemon); `copy` usa `wl-clipboard-rs`
  (protocolo ext-data-control — suportado pelo Mutter no GNOME 46+ ✅).

### 3.6 Configuração

`~/.config/quickwhisper/config.toml` (serde + toml):

```toml
[hotkey]
key = "F12"            # trigger preferido passado ao portal
mode = "push-to-talk"  # ou "toggle" (aperta p/ iniciar, aperta p/ parar)

[audio]
device = "default"
max_recording_secs = 120

[whisper]
model = "small"
language = "auto"

[overlay]
enabled = true
monitor = "pointer"    # "pointer" | "primary" | "all"
```

Nota: o portal GlobalShortcuts trata `key` como *preferred trigger* — o GNOME mostra um diálogo
de confirmação na primeira execução e o usuário pode remapear por lá; a permissão persiste via
token de sessão guardado na config.

## 4. Layout do projeto

```
quickwhisper/
├── Cargo.toml                  # crate único, bin "quickwhisper"
├── src/
│   ├── main.rs                 # clap: dispatch de subcomandos
│   ├── daemon/
│   │   ├── mod.rs              # loop principal, máquina de estados
│   │   ├── hotkey.rs           # portal GlobalShortcuts (ashpd)
│   │   ├── audio.rs            # cpal + resample + RMS
│   │   ├── transcribe.rs       # whisper-rs (spawn_blocking)
│   │   ├── dbus.rs             # serviço io.github.alaor.QuickWhisper (zbus)
│   │   └── paste.rs            # orquestra clipboard + pedido de paste à extensão / fallbacks
│   ├── history.rs              # rusqlite: schema, queries
│   ├── config.rs               # load/save TOML, defaults
│   ├── models.rs               # download/verificação de modelos ggml
│   └── cli/                    # subcomandos history/model/config/status
├── extension/                  # extensão GNOME Shell (GJS)
│   ├── metadata.json
│   ├── extension.js            # D-Bus client, ciclo de vida
│   ├── overlay.js              # pílula, onda, animações
│   └── stylesheet.css
├── data/
│   ├── quickwhisper.service    # systemd user unit
│   └── io.github.alaor.QuickWhisper.desktop
├── PLAN.md                     # este arquivo
└── README.md
```

**Crates principais:** `tokio`, `zbus`, `ashpd` (portais), `cpal`, `rubato`, `whisper-rs`,
`rusqlite`, `clap`, `serde`/`toml`, `directories`, `wl-clipboard-rs`, `thiserror`/`anyhow`,
`tracing`, `notify-rust` (notificações), `reqwest` (download de modelos).

**Dependências de sistema (Rocky 10):** `cmake`, `clang` (build do whisper.cpp), `alsa-lib-devel`,
possivelmente `openssl-devel`. Documentar no README.

## 5. Interface D-Bus (contrato daemon ⇄ extensão)

Nome: `io.github.alaor.QuickWhisper` · Path: `/io/github/alaor/QuickWhisper`

```
# Sinais (daemon → extensão)
RecordingStarted()
AudioLevel(level: d)          # RMS normalizado 0.0–1.0, ~15 Hz
ProcessingStarted()
Finished(text: s)             # extensão seta clipboard + Ctrl+V
Failed(message: s)
Cancelled()

# Métodos (extensão/CLI → daemon)
GetState() → (state: s)       # "idle" | "recording" | "processing"
CancelRecording()
```

A extensão exporta apenas presença no bus (`io.github.alaor.QuickWhisper.Overlay`) para o daemon
detectar se o overlay/paste está disponível e decidir o fallback.

## 6. Fases de implementação

### Fase 0 — Spikes de risco (validar antes de construir)
> Objetivo: provar as 3 apostas do Wayland em código descartável.

- [ ] **Spike A — Hotkey:** binário mínimo com `ashpd` registrando F12 no portal GlobalShortcuts; logar `Activated`/`Deactivated`. Validar: latência e confiabilidade do par press/release no GNOME 47; comportamento com tecla segurada (auto-repeat não deve gerar múltiplos Activated).
- [ ] **Spike B — Overlay + paste:** extensão GJS "hello pill" que desenha uma pílula bottom-center via `addTopChrome` e sintetiza Ctrl+V num campo de texto com `Clutter.VirtualInputDevice`.
- [ ] **Spike C — Whisper:** medir no hardware real: tempo de transcrição de 5 s / 15 s / 30 s de fala com modelos base/small/medium (auto-detect de idioma ligado). Escolher default.

**Critério de saída:** os 3 spikes funcionando; decisões de fallback revisadas conforme resultados.

### Fase 1 — Núcleo headless (MVP utilizável)
- [ ] Scaffolding do crate, config TOML, `tracing`, `git init`.
- [ ] `daemon`: portal hotkey → captura cpal → resample → whisper-rs → **clipboard + notificação GNOME** com o texto.
- [ ] `model download` + carregamento do modelo no boot.
- [ ] Máquina de estados com os edge cases de §7.
- [ ] systemd user unit.

**Critério de saída:** segurar F12, falar, soltar → texto no clipboard + notificação. Uso diário já possível (colar manual com Ctrl+V).

### Fase 2 — Histórico + CLI
- [ ] Schema SQLite + gravação automática de cada transcrição.
- [ ] `history list/show/copy/delete/clear` com timestamps em hora local, `--json`.
- [ ] `status`.

### Fase 3 — Extensão: overlay + auto-paste
- [ ] Extensão GJS: ciclo de vida, cliente D-Bus.
- [ ] Pílula com onda reativa ao `AudioLevel` (gradiente roxo→laranja) + animação de processing + fade-out.
- [ ] Paste automático via `VirtualInputDevice`; `St.Clipboard` como setter primário quando a extensão está presente.
- [ ] Respeito a `enable-animations` e alto contraste.
- [ ] Detecção de presença da extensão no daemon + degradação p/ notificações.

**Critério de saída:** o fluxo completo da especificação do usuário, ponta a ponta.

### Fase 4 — Polimento e distribuição
- [ ] `config set/get`, modo `toggle`, escolha de dispositivo de áudio.
- [ ] Multi-monitor (`overlay.monitor = all`).
- [ ] Empacotamento: RPM (spec) p/ Rocky; extensão publicável no extensions.gnome.org (opcional).
- [ ] README com setup completo; instruções de permissão do portal.

## 7. Edge cases e regras

| Caso | Comportamento |
|---|---|
| Gravação < 1 s (toque acidental) | Descartar; overlay some com fade rápido; nada no histórico |
| Transcrição vazia/só silêncio | Não colar, não copiar, não salvar; overlay some; log em debug |
| F12 pressionado durante Processing | Ignorar; pílula pisca levemente indicando "ocupado" |
| Modelo ausente | Notificação com instrução `quickwhisper model download small`; daemon segue vivo |
| Microfone ocupado/indisponível | Notificação de erro; sinal `Failed` |
| Tecla presa / esquecida | Corte em `max_recording_secs`; transcreve o que tem |
| Extensão ausente | Fallback: clipboard + notificações (sem overlay/auto-paste) |
| Campo ativo é um terminal | Ctrl+V não cola em muitos terminais (usam Ctrl+Shift+V) — v1: texto fica no clipboard (comportamento aceitável); investigar heurística futura |
| Sessão X11 (não é o alvo) | Detectar e avisar; não suportado na v1 |
| Daemon reiniciado no meio | Estado sempre reconstruível; extensão re-conecta via watch de nome D-Bus |

## 8. Riscos e mitigação

| Risco | Prob. | Mitigação |
|---|---|---|
| Portal GlobalShortcuts não entrega `Deactivated` confiável p/ push-to-talk | média | Spike A decide; plano B: modo `toggle` como default; plano C: leitor evdev (usuário no grupo `input`) |
| `VirtualInputDevice` bloqueado em versão futura do Shell | baixa | Fallback portal RemoteDesktop + libei (crate `reis`); ou ydotool |
| Latência do whisper irritante em CPU | média | Spike C mede; default `small`; oferecer `base`; considerar quantização (Q5) e `n_threads` tunado |
| Extensão quebra a cada versão do GNOME | alta (fato da vida) | Extensão mínima e isolada; toda lógica no daemon; CI de teste manual por release do GNOME |
| Auto-detect de idioma erra em frases curtas | média | Já aceito pelo usuário; config `language` permite fixar pt/en |

## 9. Referências para a implementação

- Skills instaladas a carregar por fase: `rust-best-practices`, `rust-async-patterns` (Fases 1–2),
  `developing-gtk-apps`/`designing-gnome-ui` (Fase 3 — HIG, animações, acessibilidade).
- Portal GlobalShortcuts: https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.GlobalShortcuts.html
- Modelos ggml: https://huggingface.co/ggerganov/whisper.cpp
- OSD nativo do GNOME Shell (referência de estilo/técnica): `js/ui/osdWindow.js` no gnome-shell.
```
