# Test Support Design

このディレクトリは integration test と TUI test の共通補助を置く場所です。
MVP では、ここを起点に「毎回クリーンな Git repo を作る」「不安定値を固定する」「外部依存を fake に置き換える」を徹底します。

## Planned modules

- `repo.rs`
  - temporary directory を作る
  - `git init` する
  - `user.name`, `user.email` を設定する
  - `.forum/` と `.git/forum/` の状態確認を助ける
- `env.rs`
  - `HOME`, `XDG_CONFIG_HOME`, `GIT_CONFIG_NOSYSTEM=1` を隔離する
  - test ごとの環境変数を注入する
- `cli.rs`
  - `git-forum` バイナリを起動する
  - stdout / stderr / exit code を検証しやすくする
- `clock.rs`
  - 固定時刻または step clock を提供する
- `ids.rs`
  - 固定 ID generator / predictable sequence を提供する
- `git.rs`
  - test 用の commit 作成
  - `GIT_AUTHOR_DATE`, `GIT_COMMITTER_DATE` の固定
  - branch 作成と merge の補助
- `ai.rs`
  - fake provider
  - 固定 run result / tool call / confidence を返す
- `tui.rs`
  - TUI backend を組み立てる
  - 一覧 / 詳細 render のテスト入力を組み立てる

## Planned sibling directories

- `tests/fixtures/`
  - import / export 用の固定ファイル
  - replay / merge の再現入力
- `tests/snapshots/`
  - `show`
  - `verify`
  - export
  - TUI render

## Rules

- global Git config に依存しない
- ネットワークを使わない
- test は commit hash や timestamp を直接 snapshot しない
- integration test は互いに状態を共有しない
- TUI test は完全なキー入力自動化ではなく render 結果の固定を優先する
