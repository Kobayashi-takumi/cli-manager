# CLI Manager

TUI ベースのターミナルマルチプレクサ。複数の CLI プロセス（例: Claude Code）を擬似端末（PTY）で管理し、2 ペインの TUI インターフェースで切り替えながら操作できる、"CLI プロセス用ウィンドウマネージャ" です。

## 目次

- [機能](#機能)
- [必要環境](#必要環境)
- [インストール](#インストール)
- [クイックスタート](#クイックスタート)
- [操作方法](#操作方法)
  - [キーバインド一覧](#キーバインド一覧)
  - [クイックスイッチャー](#クイックスイッチャー)
  - [ミニターミナル](#ミニターミナル)
  - [プレフィックスキーの仕組み](#プレフィックスキーの仕組み)
- [UI レイアウト](#ui-レイアウト)
- [ターミナルのライフサイクル](#ターミナルのライフサイクル)
- [アーキテクチャ](#アーキテクチャ)
  - [レイヤー構成](#レイヤー構成)
  - [データフロー](#データフロー)
  - [ディレクトリ構成](#ディレクトリ構成)
- [開発](#開発)
  - [ビルド & テスト](#ビルド--テスト)
  - [テスト構成](#テスト構成)
- [技術スタック](#技術スタック)
- [ライセンス](#ライセンス)

## 機能

| 機能 | 説明 |
|------|------|
| ターミナル作成 | 新しいシェルセッションを PTY 上で起動 |
| ターミナル切替 | 番号指定・前後移動でアクティブターミナルを切り替え |
| リアルタイム出力 | ANSI カラー/エスケープシーケンスを解釈して描画 |
| サイドバー | 全ターミナルの一覧・ステータス・動的 CWD を常時表示 |
| ターミナル削除 | 実行中プロセスの場合は確認ダイアログ付き |
| プレフィックスキー | tmux ライクな `Ctrl+b` プレフィックスモデル |
| 256 色 & 属性 | xterm-256color、太字/イタリック/下線/取り消し線/反転/薄字 |
| 代替画面バッファ | vim 等のフルスクリーンアプリケーション対応 |
| スクロールリージョン | DECSTBM による部分スクロール |
| ワイド文字 | CJK 文字（全角）の正確な表示 |
| アプリケーションカーソルキー | DECCKM モード対応 |
| ブラケットペースト | ペースト時のエスケープシーケンスラッピング |
| OSC 7 動的 CWD | シェルの現在ディレクトリをサイドバーに反映 |
| 通知 | BEL / OSC 9 / OSC 777 検出 → サイドバーマーク + macOS デスクトップ通知 |
| スクロールバック | 出力履歴を vim ライクなキーバインドでスクロール閲覧（10,000 行バッファ） |
| リネーム | ターミナル名を後から変更可能 |
| メモ | 各ターミナルに複数行メモを付与・編集。サイドバーに `[≡]` インジケータ表示 |
| ヘルプオーバーレイ | `Ctrl+b` → `?` でキーバインド一覧をオーバーレイ表示 |
| クイックスイッチャー | `Ctrl+b` → `f` でファジー検索オーバーレイ。名前・CWD・メモで絞り込み即座に切替 |
| ミニターミナル | フッター型クイックシェル。`` Ctrl+b `` → `` ` `` でトグル。スクロールバック対応 |

## 必要環境

- **Rust** 1.85.0 以上（edition 2024 / let-chains 構文のため）
- **OS**: macOS（Linux は将来対応予定）

## インストール

```bash
git clone <repository-url>
cd cli_manager
cargo install --path .
```

`~/.cargo/bin/cm` にインストールされます。`~/.cargo/bin` にパスが通っていれば、どこからでも `cm` で起動できます。

手動でビルドする場合:

```bash
cargo build --release
```

ビルド後のバイナリは `target/release/cm` に生成されます。

## クイックスタート

```bash
# インストール済みの場合
cm

# または cargo 経由で起動
cargo run
```

起動すると TUI が表示されます。最初のターミナルを作成するには `Ctrl+b` → `c` を押してください。

## 操作方法

### キーバインド一覧

すべての操作コマンドは **プレフィックスキー `Ctrl+b`** の後に入力します。

| キーバインド | アクション |
|---|---|
| `Ctrl+b` → `c` | 新しいターミナルを作成 |
| `Ctrl+b` → `d` | アクティブターミナルを削除（実行中なら確認あり） |
| `Ctrl+b` → `n` | 次のターミナルを選択 |
| `Ctrl+b` → `p` | 前のターミナルを選択 |
| `Ctrl+b` → `1`〜`9` | 番号指定でターミナルをジャンプ |
| `Ctrl+b` → `f` | クイックスイッチャーを開く（ファジー検索で切替） |
| `Ctrl+b` → `Ctrl+b` | 子プロセスに `Ctrl+b` を送信 |
| `Ctrl+b` → `[` | スクロールバックモードに入る |
| `Ctrl+b` → `r` | アクティブターミナルをリネーム |
| `Ctrl+b` → `m` | メモを編集 |
| `Ctrl+b` → `` ` `` | ミニターミナルのトグル（開く/閉じる/フォーカス切替） |
| `Ctrl+b` → `?` | ヘルプオーバーレイを表示 |
| `Ctrl+b` → `q` | アプリケーション終了 |
| その他のキー | アクティブターミナルの stdin へパススルー |

#### スクロールバックモード

`Ctrl+b` → `[` でスクロールバックモードに入ると、出力履歴を遡って閲覧できます。vim ライクなキーバインドで操作します。

| キーバインド | アクション |
|---|---|
| `↑` / `k` | 1 行上にスクロール |
| `↓` / `j` | 1 行下にスクロール |
| `PageUp` | 1 ページ上にスクロール |
| `PageDown` | 1 ページ下にスクロール |
| `g` | バッファの先頭にジャンプ |
| `G` | バッファの末尾にジャンプ |
| `Esc` / `q` | スクロールバックモードを終了 |

#### リネーム

`Ctrl+b` → `r` でリネームダイアログが開きます。現在の名前がプリセットされた状態で編集でき、`Enter` で確定、`Esc` でキャンセルします。

#### メモ編集

`Ctrl+b` → `m` でメモ編集オーバーレイが開きます。各ターミナルにメモを付けて用途や作業内容を記録できます。

| キーバインド | アクション |
|---|---|
| 文字入力 | テキストを入力 |
| `Ctrl+J` | 改行を挿入 |
| `↑` / `↓` | カーソルを行間移動 |
| `←` / `→` | カーソルを左右移動 |
| `Backspace` | 文字削除（行頭では前行と結合） |
| `Enter` | メモを保存して閉じる |
| `Esc` | 変更を破棄して閉じる |

メモが存在するターミナルにはサイドバーに `[≡]` インジケータが表示されます。メモはセッション中のみ保持されます。

#### ミニターミナル

`Ctrl+b` → `` ` `` でフッター領域にミニターミナルを開きます。メインターミナルを操作しながら、ちょっとしたコマンドを実行するのに便利です。

- **3 ステートトグル:** 1 回目で開く → 2 回目でメインにフォーカスを戻す → 3 回目で閉じる
- **独立した PTY:** メインターミナルとは別のシェルセッション（`$SHELL` を起動）
- **スクロールバック対応:** `Ctrl+b` → `[` でスクロールバックモードに入り、メインと同じキーバインドで履歴を閲覧可能
- **OSC 7 CWD:** ミニターミナルも動的 CWD に対応
- **自動クリーンアップ:** ミニターミナル内のプロセスが終了すると自動的に閉じる

#### クイックスイッチャー

`Ctrl+b` → `f` でクイックスイッチャーオーバーレイが表示されます。ターミナル一覧をファジー検索で絞り込み、素早く切り替えられます。VS Code の `Ctrl+P` や tmux の `choose-tree` に相当する機能です。

| キーバインド | アクション |
|---|---|
| 文字入力 | インクリメンタルにフィルタ |
| `↑` / `Ctrl+k` | 選択カーソルを上に移動 |
| `↓` / `Ctrl+j` | 選択カーソルを下に移動 |
| `Enter` | 選択ターミナルに切り替え |
| `Esc` | キャンセル（何も変更しない） |

**検索対象:** ターミナル ID、名前、動的 CWD、メモ。マッチした文字は Cyan + Bold でハイライト表示されます。

#### ヘルプオーバーレイ

`Ctrl+b` → `?` でヘルプオーバーレイが表示されます。全キーバインドを TERMINAL / NAVIGATION / SCROLLBACK / GENERAL の 4 カテゴリに分類して一覧表示します。`?` または `Esc` で閉じます。

### プレフィックスキーの仕組み

`Ctrl+b` は tmux と同じプレフィックスキーです。InputHandler が以下のステートマシンで管理します。

```mermaid
stateDiagram-v2
    [*] --> Normal
    Normal --> PrefixWait : Ctrl+b 押下
    PrefixWait --> Normal : コマンドキー入力\n(c/d/n/p/1-9/q)
    PrefixWait --> ScrollbackMode : [ 押下
    PrefixWait --> DialogInput : r 押下 (リネーム)
    PrefixWait --> MemoEdit : m 押下 (メモ編集)
    PrefixWait --> DialogInput : f 押下 (クイックスイッチャー)
    PrefixWait --> HelpView : ? 押下 (ヘルプ)
    PrefixWait --> MiniTerminalInput : ` 押下 (ミニターミナル)
    HelpView --> Normal : ? / Esc 押下
    MiniTerminalInput --> Normal : Ctrl+b → ` (閉じる)
    MiniTerminalInput --> PrefixWait : Ctrl+b 押下
    PrefixWait --> Normal : 1秒タイムアウト\n(Ctrl+b を子プロセスへ送信)
    PrefixWait --> Normal : Ctrl+b 再押下\n(Ctrl+b を子プロセスへ送信)
    ScrollbackMode --> Normal : Esc / q 押下
    DialogInput --> Normal : Enter / Esc
    MemoEdit --> Normal : Enter (保存) / Esc (破棄)
```

**ポイント:**
- `Ctrl+b` を押すと 1 秒間コマンド入力を待機します
- 1 秒以内にコマンドキーを押さなかった場合、`Ctrl+b` が子プロセスにそのまま送られます
- 子プロセスに `Ctrl+b` 自体を送りたい場合は `Ctrl+b` → `Ctrl+b` と 2 回押します

## UI レイアウト

2 ペイン構成のインターフェースです。`Ctrl+b` → `` ` `` でフッター領域にミニターミナルが追加されます。

```
┌───────────────────────┬────────────────────────────────────┐
│ Terminals          3  │                                    │
│───────────────────────│ ~/projects/my-app                  │
│ ● 1: my-app          │ $ claude "テストを書いて"            │
│   /projects/my-app    │                                    │
│   claude running      │ 了解です。テストを作成します。        │
│───────────────────────│ src/lib.rs を読んでいます...         │
│ ○ 2: api-srv [≡]     │                                    │
│   /projects/api       │ テストを書きました：                 │
│   idle                │ - test_create_task                  │
│───────────────────────│ - test_delete_task                  │
│ ✗ 3: frontend      * │                                    │
│   /projects/front     │                                    │
│   exited (0)          │                                    │
│                       │                                    │
│───────────────────────│                                    │
│ ^b ?:Help q:Quit      │ $ _                                │
│                       ├────────────────────────────────────┤
│                       │ Mini Terminal          ~/projects  │
│                       │ $ git status                       │
│                       │ On branch main                     │
│                       │ $ _                                │
└───────────────────────┴────────────────────────────────────┘
  ← サイドバー (25文字) →  ← メインペイン (残り幅) →
                                  ↑ ミニターミナル (高さ10行)
```

```mermaid
block-beta
    columns 2
    block:sidebar:1
        columns 1
        A["サイドバー (25文字固定)"]
        B["ターミナル一覧"]
        C["ステータスアイコン"]
        D["通知マーク (*) / メモマーク ([≡])"]
        E["ヘルプバー"]
    end
    block:main:1
        columns 1
        F["メインペイン (残り幅)"]
        G["アクティブターミナル出力"]
        H["ANSI カラー / 256 色対応"]
        I["ワイド文字・カーソル表示"]
        J["ミニターミナル (高さ10行)"]
    end
```

### サイドバーのステータスアイコン

| アイコン | ステータス | 意味 |
|---------|-----------|------|
| `●` | Running | プロセス実行中 |
| `○` | Idle | アイドル状態 |
| `✗` | Exited | プロセス終了済み（出力は保持） |
| `*` | 通知あり | 未読通知（BEL / OSC 9 / OSC 777） |
| `[≡]` | メモあり | ターミナルにメモが付与されている |

## ターミナルのライフサイクル

ターミナルは以下の状態遷移で管理されます。

```mermaid
stateDiagram-v2
    [*] --> Created : Ctrl+b → c
    Created --> Running : シェルプロセス起動
    Running --> Exited : プロセス終了\n(出力は保持)
    Exited --> Removed : Ctrl+b → d
    Running --> Removed : Ctrl+b → d\n(確認ダイアログ後)
    Removed --> [*]

    note right of Running
        キー入力はアクティブターミナルの
        stdin へパススルーされる
    end note

    note right of Exited
        出力バッファは保持され
        閲覧可能
    end note
```

**状態の説明:**

| 状態 | 説明 |
|------|------|
| **Created** | ターミナル作成直後。PTY が割り当てられる |
| **Running** | シェルプロセスが実行中。キー入力を受け付ける |
| **Exited** | プロセスが終了。出力は保持され閲覧可能 |
| **Removed** | ユーザーが明示的に削除。リストから除去される |

## アーキテクチャ

クリーンアーキテクチャに基づき、厳密な依存方向を維持しています。

### レイヤー構成

```mermaid
graph TD
    subgraph "Infrastructure 層"
        MAIN["main.rs<br/>(DI アセンブリ)"]
        PTY["PortablePtyAdapter<br/>(portable-pty)"]
        VTE["VteScreenAdapter<br/>(vte)"]
        VT100["Vt100ScreenAdapter<br/>(vt100)"]
        TUI["TUI<br/>(ratatui + crossterm)"]
        INPUT["InputHandler"]
        WIDGETS["Widgets<br/>(sidebar, terminal_view,<br/>mini_terminal_view, dialog,<br/>memo_overlay, help_overlay,<br/>quick_switcher)"]
        NOTIF["MacOsNotifier<br/>(notify-rust)"]
    end

    subgraph "Interface Adapter 層"
        CTRL["TuiController"]
        PTYPORT["PtyPort trait"]
        SCRPORT["ScreenPort trait"]
        FACTORY["Adapter Factories"]
    end

    subgraph "Usecase 層"
        UC["TerminalUsecase&lt;P, S&gt;"]
    end

    subgraph "Domain 層"
        ENT["ManagedTerminal"]
        VO["Value Objects<br/>(TerminalId, TerminalSize,<br/>Cell, CursorPos, Color,<br/>NotificationEvent)"]
    end

    subgraph "Shared"
        ERR["AppError"]
    end

    MAIN --> CTRL
    MAIN --> FACTORY
    FACTORY --> PTY
    FACTORY --> VTE
    FACTORY --> VT100
    TUI --> CTRL
    INPUT --> CTRL
    CTRL --> UC
    PTY -.->|implements| PTYPORT
    VTE -.->|implements| SCRPORT
    VT100 -.->|implements| SCRPORT
    UC --> PTYPORT
    UC --> SCRPORT
    UC --> ENT
    ENT --> VO
    NOTIF -.-> UC

    style MAIN fill:#4a9eff,color:#fff
    style PTY fill:#4a9eff,color:#fff
    style VTE fill:#4a9eff,color:#fff
    style VT100 fill:#4a9eff,color:#fff
    style TUI fill:#4a9eff,color:#fff
    style INPUT fill:#4a9eff,color:#fff
    style WIDGETS fill:#4a9eff,color:#fff
    style NOTIF fill:#4a9eff,color:#fff
    style CTRL fill:#7c4dff,color:#fff
    style PTYPORT fill:#7c4dff,color:#fff
    style SCRPORT fill:#7c4dff,color:#fff
    style FACTORY fill:#7c4dff,color:#fff
    style UC fill:#00bfa5,color:#fff
    style ENT fill:#ff6d00,color:#fff
    style VO fill:#ff6d00,color:#fff
    style ERR fill:#78909c,color:#fff
```

**依存方向:** `Infrastructure → Interface Adapter → Usecase → Domain`（内側ほど依存が少ない）

| レイヤー | 責務 | 外部クレート依存 |
|---------|------|----------------|
| **Domain** | エンティティ・値オブジェクト | なし（純粋 Rust） |
| **Usecase** | ターミナル管理ロジック | なし（ポートトレイトのみ） |
| **Interface Adapter** | ポートトレイト定義・コントローラ・ファクトリ | なし |
| **Infrastructure** | 具象実装（PTY, 画面, TUI, 通知, DI） | ratatui, crossterm, portable-pty, vte, vt100, notify-rust |
| **Shared** | エラー型 | thiserror |

### データフロー

ユーザー入力から画面描画までの一連の流れです。

```mermaid
sequenceDiagram
    participant User as ユーザー
    participant Input as InputHandler
    participant Ctrl as TuiController
    participant UC as TerminalUsecase
    participant PTY as PtyPort
    participant Screen as ScreenPort
    participant TUI as ratatui

    Note over User,TUI: キー入力フロー
    User->>Input: KeyEvent
    Input->>Input: ステートマシン判定<br/>(Normal / PrefixWait)
    Input->>Ctrl: AppAction
    Ctrl->>UC: create / delete / select / write

    alt ターミナル作成
        UC->>PTY: spawn(command, size)
    else キー入力転送
        UC->>PTY: write(terminal_id, data)
    end

    Note over User,TUI: PTY 出力フロー
    loop 50ms ポーリング
        UC->>PTY: poll_all()
        PTY-->>UC: stdout データ
        UC->>Screen: process(terminal_id, data)
        Screen->>Screen: VTE パース → セルグリッド更新
        UC->>Screen: get_cells(terminal_id)
        Screen-->>TUI: &Vec<Vec<Cell>>
        TUI->>User: 画面描画
    end

    Note over User,TUI: 通知フロー
    Screen-->>UC: drain_notifications()
    UC->>UC: 通知を ManagedTerminal に記録
    UC-->>TUI: take_pending_notifications()
    TUI->>User: サイドバーに * マーク表示
    TUI->>User: macOS デスクトップ通知
```

### ディレクトリ構成

```
src/
├── main.rs                              # DI アセンブリ & エントリーポイント
├── domain/                              # Domain 層
│   ├── model/
│   │   └── terminal.rs                  # ManagedTerminal エンティティ
│   └── primitive/                       # 値オブジェクト
│       ├── terminal_id.rs               # TerminalId
│       ├── terminal_status.rs           # TerminalStatus
│       ├── terminal_size.rs             # TerminalSize
│       ├── cell.rs                      # Cell, CursorPos, Color
│       └── notification.rs              # NotificationEvent (Bell/Osc9/Osc777)
├── usecase/
│   └── terminal_usecase.rs              # TerminalUsecase<P: PtyPort, S: ScreenPort>
├── interface_adapter/                   # Interface Adapter 層
│   ├── port/
│   │   ├── pty_port.rs                  # PtyPort トレイト
│   │   └── screen_port.rs              # ScreenPort トレイト
│   ├── adapter/
│   │   ├── pty_adapter_factory.rs       # PTY アダプタファクトリ
│   │   └── screen_adapter_factory.rs    # Screen アダプタファクトリ
│   └── controller/
│       └── tui_controller.rs            # TuiController (AppAction ディスパッチ)
├── infrastructure/                      # Infrastructure 層
│   ├── pty/
│   │   └── portable_pty_adapter.rs      # PtyPort 実装 (portable-pty)
│   ├── screen/
│   │   ├── vte_screen.rs               # ScreenPort 実装 (vte)
│   │   ├── vt100_screen.rs             # ScreenPort 実装 (vt100)
│   │   └── osc7.rs                     # OSC 7 URI パーサー
│   ├── tui/
│   │   ├── app_runner.rs                # メインイベントループ
│   │   ├── input.rs                     # InputHandler (キー入力処理)
│   │   ├── fuzzy_matcher.rs             # ファジーマッチエンジン (クイックスイッチャー用)
│   │   └── widgets/                     # UI ウィジェット
│   │       ├── layout.rs                # 2ペインレイアウト
│   │       ├── sidebar.rs               # サイドバー (ターミナル一覧 + 通知マーク)
│   │       ├── terminal_view.rs         # メインペイン (出力表示 + ワイド文字)
│   │       ├── mini_terminal_view.rs   # ミニターミナル (フッター型クイックシェル)
│   │       ├── dialog.rs                # 確認・リネームダイアログ
│   │       ├── memo_overlay.rs          # メモ編集オーバーレイ
│   │       ├── help_overlay.rs          # ヘルプオーバーレイ
│   │       └── quick_switcher.rs        # クイックスイッチャーオーバーレイ
│   └── notification/
│       └── macos_notifier.rs            # macOS デスクトップ通知 (notify-rust)
└── shared/
    └── error.rs                         # AppError enum
```

## 開発

### ビルド & テスト

```bash
# 型チェック
cargo check

# ビルド
cargo build

# テスト（全 873 件）
cargo test

# 特定のテストのみ実行
cargo test test_create_terminal

# Lint
cargo clippy
```

### テスト構成

合計 **873** ユニットテスト。各モジュールごとの内訳は以下の通りです。

```mermaid
pie title ユニットテスト構成 (873件)
    "VteScreenAdapter (177)" : 177
    "AppRunner (129)" : 129
    "Vt100ScreenAdapter (103)" : 103
    "InputHandler (96)" : 96
    "TerminalUsecase (61)" : 61
    "QuickSwitcher (40)" : 40
    "TuiController (38)" : 38
    "Sidebar (32)" : 32
    "MiniTerminalView (31)" : 31
    "TerminalView (31)" : 31
    "その他 (135)" : 135
```

| モジュール | テスト数 | テスト対象 |
|-----------|---------|-----------|
| `VteScreenAdapter` | 177 | ANSI パース、セルグリッド、カーソル移動、代替画面、スクロールリージョン、ワイド文字、OSC タイトル、通知 |
| `AppRunner` | 129 | イベントループシミュレーション、スクロールバック（メイン/ミニ）、フォーカス制御、ミニターミナル管理、クイックスイッチャー統合 |
| `Vt100ScreenAdapter` | 103 | vt100 ベースパース、セル属性、OSC 7 CWD、OSC タイトル、通知、スクロールバック、カーソルスタイル、DSR 応答 |
| `InputHandler` | 96 | ステートマシン、プレフィックスキー、タイムアウト、アプリケーションカーソルキー、ブラケットペースト、スクロールバックモード、メモ編集モード、ヘルプ表示、ミニターミナル入力 |
| `TerminalUsecase` | 61 | CRUD 操作、ポーリング、通知収集、リネーム、メモ操作、エラーハンドリング |
| `QuickSwitcher` | 40 | オーバーレイ描画、クエリ入力、選択ハイライト、マッチ文字ハイライト、スクロール、小画面対応 |
| `TuiController` | 38 | AppAction ディスパッチ、状態管理、リネーム・メモ・ヘルプ・ミニターミナル・クイックスイッチャー操作 |
| `Sidebar` | 32 | ターミナル一覧描画、動的 CWD 表示、通知マーク、メモインジケータ |
| `MiniTerminalView` | 31 | ミニターミナル描画、セルグリッド、ワイド文字、カーソル位置、スクロールバック表示 |
| `TerminalView` | 31 | 出力表示、ワイド文字クリッピング、カーソル位置、スクロールバック表示 |
| `ManagedTerminal` | 17 | エンティティ操作、通知フラグ、リネーム、メモ |
| `MacOsNotifier` | 17 | デスクトップ通知送信、レート制限 |
| `HelpOverlay` | 16 | ヘルプオーバーレイ描画、カテゴリ表示、キーバインド一覧、クイックスイッチャー表示、小画面対応 |
| `NotificationEvent` | 15 | Bell/Osc9/Osc777 イベント |
| `FuzzyMatcher` | 13 | サブシーケンスマッチ、スコアリング、フィルタ＆ソート、日本語、エッジケース |
| `Dialog` | 11 | 確認・リネームダイアログ描画 |
| `Layout` | 10 | 2ペインレイアウト計算、ミニターミナル分割 |
| `OSC 7 Parser` | 10 | URI パース、パーセントデコード |
| `Cell` | 5 | セル属性、色 |
| `MemoOverlay` | 3 | メモ編集オーバーレイ描画 |

**モックパターン（スレッドセーフ）:**
- `MockPtyPort`: `Arc<Mutex<>>` で呼び出し履歴を追跡（Send+Sync 対応）
- `MockScreenPort`: `&mut self` メソッドのため plain フィールドで安全

## 技術スタック

| クレート | バージョン | 用途 |
|---------|-----------|------|
| [ratatui](https://ratatui.rs/) | 0.30 | TUI フレームワーク |
| [crossterm](https://github.com/crossterm-rs/crossterm) | 0.29 | ターミナルバックエンド（bracketed-paste feature 有効） |
| [portable-pty](https://github.com/wez/wezterm/tree/main/pty) | 0.9 | PTY 管理 |
| [vte](https://github.com/alacritty/vte) | 0.15 | ANSI エスケープパーサー |
| [vt100](https://github.com/doy/vt100-rust) | 0.16 | VT100 ターミナルエミュレータ（代替 ScreenPort 実装） |
| [unicode-width](https://github.com/unicode-rs/unicode-width) | 0.2 | ワイド文字（CJK 等）の表示幅判定 |
| [notify-rust](https://github.com/hoodie/notify-rust) | 4 | macOS デスクトップ通知 |
| [thiserror](https://github.com/dtolnay/thiserror) | 2.0 | エラー型定義 |
| [anyhow](https://github.com/dtolnay/anyhow) | 1.0 | エラー伝播 |
| [libc](https://github.com/rust-lang/libc) | 0.2 | 低レベル PTY 操作（non-blocking I/O） |

## ライセンス

MIT
