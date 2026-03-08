# MVP TODO

この文書は [doc/spec/MVP_SPEC.md](./spec/MVP_SPEC.md) を、MVP 完了までのマイルストーンに再編したものです。
仕様の解釈が分かれた場合は spec を優先します。

## Finish line

- [x] 空の Git repository で `git forum init` が動く
- [x] `issue` / `rfc` / `decision` を作成できる
- [x] 型付き発言を追加できる
- [x] AI run provenance を保存できる
- [x] policy による state transition 検証が動く
- [x] evidence を追加できる
- [x] `git forum show` で open objections / latest summary / timeline を表示できる
- [x] `git forum verify` で最低限の guard を評価できる
- [ ] `git forum reindex` で index を再構築できる
- [ ] branch で分岐した thread を最小限 merge できる
- [ ] `git forum tui` で一覧・詳細・基本フィルタを操作できる
- [x] Rust stable toolchain で build / test できる

## Test harness baseline

全マイルストーンを通して、次の testing strategy を維持する。

- [x] unit tests は replay / state machine / policy / guard / merge / index / search を pure Rust で検証する
- [x] integration tests は毎回 temporary Git repo を作り、global/system Git config に依存しない
- [ ] AI integration は mock / fake provider で検証できる
- [x] clock と ID generator は差し替え可能にする
- [ ] snapshot 比較は `show` / `verify` / export / TUI render のような安定した出力面に限定する
- [x] CI の最低ラインは `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test`

Initial layout:

- [x] `tests/support/` に一時 repo、環境隔離、Git helper、clock/id stub、CLI helper を置く
- [ ] `tests/fixtures/` に import/export や replay 用の固定入力を置く
- [ ] `tests/snapshots/` に stable output の snapshot を置く

## Milestone 0: Rust bootstrap

Goal:
Rust で継続実装できる最小の土台を作る。

Done when:

- [x] `Cargo.toml` と `src/` の最小構成がある
- [x] `git-forum` 単一バイナリの entrypoint がある
- [x] `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` が通る
- [x] エラー型、設定読込、CLI entrypoint の基本骨格がある
- [x] `tests/support/` の骨格を作る
- [x] test 用の clock / ID generator 差し替えポイントを設計する

Verification:

```bash
cargo build
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

## Milestone 1: Repository and event foundation

Goal:
Git repository の中に forum データを保存し、event replay で状態を復元できるようにする。

Includes:

- [x] `.forum/` と `.git/forum/` の初期化
- [x] `git forum init`
- [x] thread / run / actor / index の ref namespace
- [x] event を Git commit として保存する処理
- [x] event replay による thread 状態再構成
- [x] `git forum doctor`
- [x] `git forum reindex` の骨格
- [x] isolated temporary Git repo を作る integration test helper
- [x] Git config を隔離する test helper

Exit criteria:

- [x] 空 repo で `git forum init` が成功する
- [x] thread の最新状態を ref から再構成できる
- [x] local index を壊しても再構築できる前提が成立している
- [x] integration test が global/system Git config に依存せず通る

Verification:

```bash
cargo run -- forum init
cargo run -- forum doctor
cargo run -- forum reindex
```

## Milestone 2: Thread lifecycle and read path

Goal:
thread の作成、一覧、詳細表示ができ、基本的な議論の流れを CLI で見られるようにする。

Includes:

- [x] `Thread`, `Event`, `Evidence`, `Actor`, `Run`, `Approval` の Rust 型
- [x] issue / rfc / decision の state machine
- [x] 人間可読 ID 採番
- [x] `git forum issue new`
- [x] `git forum rfc new`
- [x] `git forum decision new`
- [x] thread の初期 body を保存できる
- [x] `git forum ls` と kind 別 `ls`
- [x] `git forum show`
- [x] `show` 出力の snapshot 方針を固める

Exit criteria:

- [x] `issue` / `rfc` / `decision` を作成できる
- [x] `git forum show` で title / body / kind / state / timeline を表示できる
- [x] `show` の表示が replay 結果と一致する
- [x] `show` の snapshot が不安定値なしで比較できる

Verification:

```bash
cargo run -- forum issue new "First issue" --body "Problem statement"
cargo run -- forum rfc new "First RFC" --body-file ./tmp/rfc-body.md
cargo run -- forum decision new "First decision"
cargo run -- forum ls
cargo run -- forum show RFC-0001
```

## Milestone 3: Structured discussion and approvals

Goal:
型付き議論、node の解決状態、policy に基づく state change を実装する。

Includes:

- [x] `claim`, `question`, `objection`, `alternative`, `evidence`, `summary`, `decision`, `action`, `risk`, `assumption`
- [x] `git forum say`
- [x] `git forum revise`
- [x] `git forum retract`
- [x] `git forum resolve`
- [x] `git forum reopen`
- [x] open objections / open actions の算出
- [x] `.forum/policy.toml` parser
- [ ] role ごとの node type / state transition 制約
- [ ] provenance 必須判定
- [x] `one_human_approval`, `at_least_one_summary`, `no_open_objections`
- [x] `git forum state <THREAD_ID> <NEW_STATE> [--sign <ACTOR_ID>]...`
- [x] `git forum verify`
- [x] `git forum policy lint`
- [x] `git forum policy check`
- [x] `verify` 出力の snapshot または安定比較を用意する

Exit criteria:

- [x] 型付き発言を追加できる
- [x] `objection` と `action` を resolve / reopen できる
- [x] `accepted` への遷移で human approval と guard が評価される
- [x] `git forum show` で open objections / latest summary / timeline を表示できる
- [x] `git forum verify` で最低限の guard を評価できる
- [x] policy / guard の unit test が揃う

Verification:

```bash
cargo run -- forum say RFC-0001 --type claim --body "Needed for compatibility."
cargo run -- forum say RFC-0001 --type objection --body "Benchmarks are missing."
cargo run -- forum policy lint
cargo run -- forum policy check RFC-0001 --transition under-review->accepted
cargo run -- forum state RFC-0001 accepted --sign human/alice
cargo run -- forum verify RFC-0001
cargo run -- forum show RFC-0001
```

## Milestone 4: Evidence and AI provenance

Goal:
根拠リンクと AI run の provenance を thread に結びつける。

Includes:

- [x] `git forum evidence add`
- [x] `commit`, `file`, `hunk`, `test`, `benchmark`, `doc`, `thread`, `external` の evidence
- [x] `git forum link`
- [x] detail view と timeline から evidence / relation を辿れる表示
- [x] actor と run を分離した保存モデル
- [x] `git forum run spawn`
- [x] `git forum run ls`
- [x] `git forum run show`
- [x] `model`, `context_refs`, `tool_calls`, `result`, `confidence` の保持
- [ ] AI actor 書き込み時の policy / provenance 検証
- [ ] fake AI provider による integration test

Exit criteria:

- [x] evidence を追加できる
- [x] AI run provenance を保存できる
- [x] `run show` で provenance を追える
- [x] evidence と AI run が thread detail から辿れる
- [ ] ネットワークなしで AI 関連テストが通る (fake provider は未実装)

Verification:

```bash
cargo run -- forum evidence add RFC-0001 --kind benchmark --ref bench/result.csv
cargo run -- forum link RFC-0001 ISSUE-0001 --rel implements
cargo run -- forum run spawn RFC-0001 --as ai/reviewer
cargo run -- forum run ls
cargo run -- forum run show RUN-0001
cargo run -- forum show RFC-0001
```

## Milestone 5: Index, search, and TUI

Goal:
実用的な閲覧速度と read-first TUI を用意する。

Includes:

- [ ] SQLite index
- [ ] `git forum reindex` による完全再構築
- [ ] index なし fallback
- [ ] title / body / label / kind / state / assignee の lexical search
- [ ] `git forum tui`
- [ ] thread 一覧表示
- [ ] kind / state の基本フィルタ
- [ ] thread detail 表示
- [ ] open objections / latest summary / timeline の TUI 表示
- [ ] refresh
- [ ] 編集操作は MVP では CLI 委譲のままにする
- [ ] TUI render test 用 backend を使ったテスト

Exit criteria:

- [ ] `git forum reindex` で index を再構築できる
- [ ] 一覧と詳細が index 経由で実用速度で表示できる
- [ ] `git forum tui` で一覧・詳細・基本フィルタを操作できる
- [ ] TUI の一覧 / 詳細表示を自動テストで固定できる

Verification:

```bash
cargo run -- forum reindex
cargo run -- forum tui
cargo run -- forum tui RFC-0001
cargo test index
```

## Milestone 6: Merge, import/export, and release hardening

Goal:
MVP の残り要件を閉じ、受け入れ条件を満たした状態で固定する。

Includes:

- [ ] 新規 `say` event 追加同士の auto-merge
- [ ] evidence 集合追加の auto-merge
- [ ] summary 追加の auto-merge
- [ ] 競合する terminal state の conflict 検出
- [ ] concurrent `resolve` / `reopen` の conflict 検出
- [ ] synthetic merge event と unresolved conflict 表示
- [ ] GitHub issue から `issue` への最小 import
- [ ] markdown RFC から `rfc` への手動補助付き import
- [ ] `issue` の GitHub issue 形式 export
- [ ] `rfc` の markdown export
- [ ] `decision` の ADR 形式 export
- [ ] replay / policy / verify / merge / index のユニットテスト
- [ ] Git repository を使う統合テスト
- [ ] import / export fixture と snapshot
- [ ] README と spec のコマンド例の整合維持
- [ ] CI を `format -> lint -> test` の順で回す
- [ ] acceptance criteria の最終確認

Exit criteria:

- [ ] branch で分岐した thread を最小限 merge できる
- [ ] import / export の最低限要件を満たす
- [ ] Rust stable toolchain で build / test できる
- [ ] Finish line のチェックがすべて埋まる

Verification:

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

## Scope guard

MVP では次を入れない。

- Web UI
- 中央サーバー
- リアルタイム共同編集
- 高度なアクセス制御
- フル編集機能を備えた rich TUI
- 自動 patch 適用
- 複雑な embeddings ベース推薦
- 企業向け PM 機能
