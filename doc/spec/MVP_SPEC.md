# git-forum MVP Specification

## 1. Purpose

この文書は `git-forum` の MVP 実装仕様を定義する。

MVP の目的は、Git repository 内で issue / RFC / decision を管理し、AI と人間の議論を **構造化・追跡可能・ローカル完結** な形で扱える CLI と簡易 TUI を成立させることにある。

MVP は「思想検証」と「使い勝手の検証」が中心であり、完全な SaaS 互換や大規模組織向け機能は含まない。

## 2. Goals

MVP が満たすべき要件は次の通り。

1. Git repository 内で `issue`, `rfc`, `decision` を作成・表示・更新できる。
2. 発言を型付きノードとして扱える。
3. すべての変更は append-only event として保存される。
4. AI actor の発言には provenance を紐づけられる。
5. 状態遷移は policy に従って検証される。
6. commit / file / test / benchmark / thread への evidence link を持てる。
7. ローカル検索と一覧表示が実用速度で動作する。
8. 単一 repository で外部サービスなしに完結する。
9. CLI に加え、一覧・詳細閲覧・基本フィルタに集中した簡易 TUI を提供する。

## 2.1 Implementation constraints

MVP の主要実装言語は Rust とする。

- Rust stable toolchain で build / test できること。
- 配布物は `git-forum` 単一バイナリを基本とすること。
- Git integration は subprocess / libgit2 系のどちらでもよいが、authoritative data の意味論は本仕様を優先すること。

## 3. Non-goals

MVP では次を対象外とする。

- Web UI
- 中央サーバー
- リアルタイム共同編集
- 高度なアクセス制御
- 完全な GitHub / GitLab 双方向同期
- 自動 patch 適用
- 複雑な embeddings ベース推薦
- 企業向け PM 機能
- フル編集機能を備えた rich TUI

## 4. Terminology

### Thread

`issue`, `rfc`, `decision` の共通オブジェクト。

### Event

Thread に対して起きた immutable な変化。

### Node

議論の論理単位。CLI で見える発言の単位。

### Evidence

議論に紐づく根拠参照。

### Actor

人間または AI の参加主体。

### Run

AI による 1 回の処理実行。

### Approval

state / decision を承認する人間の approval 記録。

## 5. Supported thread kinds

MVP では thread kind は以下の 3 種のみ。

- `issue`
- `rfc`
- `decision`

## 6. State machines

### 6.1 issue

```text
open -> triaged -> planned -> in-progress -> resolved -> verified -> closed
                 \-> blocked
                 \-> rejected
```

### 6.2 rfc

```text
draft -> proposed -> under-review -> accepted -> implemented -> verified -> archived
                           \-> changes-requested
                           \-> rejected
                           \-> superseded
```

### 6.3 decision

```text
proposed -> accepted
         \-> rejected
         \-> superseded
```

状態は event の replay から導出される。thread に対して mutable な state を唯一の真実として保存してはならない。

## 7. Data model

## 7.1 Thread

必須フィールド:

- `id`
- `kind`
- `title`
- `status`
- `created_at`
- `created_by`

任意フィールド:

- `body`
- `labels[]`
- `assignees[]`
- `scope.repo`
- `scope.branch`
- `scope.paths[]`
- `links[]`

JSON 例:

```json
{
  "id": "RFC-0012",
  "kind": "rfc",
  "title": "Trait-based solver backend",
  "body": "Needed to make plugin ABI stability explicit in the solver boundary.",
  "status": "under-review",
  "created_at": "2026-03-08T10:12:00Z",
  "created_by": "human/alice",
  "labels": ["architecture", "solver"],
  "assignees": ["human/alice", "ai/reviewer"],
  "scope": {
    "repo": true,
    "branch": null,
    "paths": ["src/solver/**"]
  }
}
```

`status` は event replay から導出される表示用フィールドであり、authoritative な唯一の真実ではない。

## 7.2 Event

必須フィールド:

- `thread_id`
- `event_type`
- `created_at`
- `actor`
- `base_rev`
- `parents[]`

条件付き必須:

- `node_type` (`event_type = say` の場合)
- `body` (`create`, `say`, `edit`, `decision` 等)
- `target_node_id` (`edit`, `retract`, `resolve`, `reopen` の場合)
- `provenance` (AI actor の場合)
- `approvals[]` (`state`, `decision` で guard が要求する場合)

許可される `event_type`:

- `create`
- `edit`
- `retract`
- `say`
- `link`
- `unlink`
- `state`
- `assign`
- `decision`
- `resolve`
- `reopen`
- `spawn`
- `result`
- `verify`
- `merge`
- `close`

## 7.3 Node type

MVP では次の node type をサポートする。

- `claim`
- `question`
- `objection`
- `alternative`
- `evidence`
- `summary`
- `decision`
- `action`
- `risk`
- `assumption`

`objection` と `action` は作成時に open とみなし、`resolve` event で closed、`reopen` event で再び open とみなす。
`retract` された node は履歴には残るが、open count や guard 判定からは除外する。

## 7.4 Evidence

必須フィールド:

- `evidence_id`
- `kind`
- `ref`

許可される `kind`:

- `commit`
- `file`
- `hunk`
- `test`
- `benchmark`
- `doc`
- `thread`
- `external`

### locator

MVP では locator を任意とする。以下を許容する。

- `path`
- `lines`
- `rows`
- `commit`
- `url`

## 7.5 Actor

必須フィールド:

- `actor_id`
- `kind` (`human` or `ai`)
- `display_name`

任意フィールド:

- `roles[]`
- `policy_profile`
- `key_id`

## 7.6 Run

AI run の必須フィールド:

- `run_id`
- `actor_id`
- `thread_id`
- `started_at`
- `ended_at`
- `model.provider`
- `model.name`
- `prompt.system_hash`
- `prompt.task_hash`
- `prompt.context_refs[]`
- `result.status`

任意フィールド:

- `tool_calls[]`
- `usage.*`
- `result.confidence`

## 7.7 Approval

必須フィールド:

- `actor_id`
- `approved_at`
- `mechanism`

任意フィールド:

- `key_id`
- `proof_ref`

MVP で必須とする `mechanism` は `recorded` のみとする。これは「誰が承認したか」を event に記録する approval であり、暗号学的検証は将来拡張とする。
GPG / SSH / Sigstore などの cryptographic signing はサポートしてよいが、MVP の必須要件ではない。

## 8. Storage layout

## 8.1 Git refs

authoritative data は以下の ref namespace に保存する。

- `refs/forum/threads/<THREAD_ID>`
- `refs/forum/runs/<RUN_ID>`
- `refs/forum/actors/<ACTOR_ID>`
- `refs/forum/index/<THREAD_ID>`

`refs/forum/index/*` は再構築可能な materialized snapshot であり、破損時に再生成できなければならない。

## 8.2 Working tree files

repo 共有対象の設定ファイル:

```text
.forum/
  policy.toml
  actors.toml
  templates/
    issue.md
    rfc.md
    decision.md
```

## 8.3 Local-only files

```text
.git/forum/
  index.sqlite
  local.toml
  logs/
```

`local.toml` は API key や local model endpoint など、共有すべきでない設定のみを保持する。

## 9. Event persistence

MVP では各 event を専用 Git commit として保存する。

### Requirements

1. commit tree には少なくとも `event.json` を含むこと。
2. commit parent は直前 event commit を参照すること。
3. merge 時は複数 parent を持つ merge event commit を作成できること。
4. thread の最新状態は `refs/forum/threads/<THREAD_ID>` が指す commit から再構成できること。

## 10. Materialization

`git forum show` や `git forum ls` の高速化のため、MVP では local index を持つ。

### Index requirements

- SQLite を使用してよい。
- authoritative data は index ではなく Git refs である。
- `git forum reindex` で完全再構築できること。
- index がなくても最悪動作可能であること。

## 11. Policy system

MVP では `.forum/policy.toml` を採用する。

### Policy responsibilities

- role ごとの許可 node type
- role ごとの許可 state transition
- AI actor に provenance を要求するか
- 特定 transition の guard 条件
- approval が必要な transition と必要人数

### Example

```toml
[roles.reviewer]
can_say = ["objection", "evidence", "summary", "risk"]
can_transition = ["under-review->changes-requested"]

[roles.maintainer]
can_say = ["claim", "decision", "summary"]
can_transition = ["draft->proposed", "under-review->accepted"]

[[guards]]
on = "under-review->accepted"
requires = ["one_human_approval", "at_least_one_summary", "no_open_objections"]
```

### MVP validation rules

最低限、以下を実装する。

1. AI actor の `say` / `state` / `decision` は provenance 必須。
2. policy にない transition は拒否。
3. `accepted` への遷移には human approval を要求できる。
4. `no_open_objections` guard を評価できる。
5. approval の最小単位として `recorded` mechanism を扱える。

## 12. CLI surface

MVP の必須コマンドは以下とする。

### Repository setup

```bash
git forum init
git forum doctor
git forum reindex
```

### Thread creation

```bash
git forum issue new <title> [--body <TEXT> | --body-file <PATH>]
git forum rfc new <title> [--body <TEXT> | --body-file <PATH>]
git forum decision new <title> [--body <TEXT> | --body-file <PATH>]
```

### Listing / display

```bash
git forum --help-llm
git forum ls
git forum issue ls
git forum rfc ls
git forum decision ls
git forum show <THREAD_ID>
git forum node show <NODE_ID>
```

### TUI

```bash
git forum tui
git forum tui <THREAD_ID>
```

### Discussion

```bash
git forum say <THREAD_ID> --type <NODE_TYPE> --body <TEXT>
git forum revise <NODE_ID> --body <TEXT>
git forum retract <NODE_ID> --reason <TEXT>
git forum resolve <NODE_ID>
git forum reopen <NODE_ID>
```

### State changes

```bash
git forum state <THREAD_ID> <NEW_STATE> [--sign <ACTOR_ID>]...
```

### Evidence / links

```bash
git forum evidence add <THREAD_ID> --kind <KIND> --ref <REF>
git forum link <FROM> <TO> --rel <REL>
```

### AI runs

```bash
git forum run spawn <THREAD_ID> --as <ACTOR_ID>
git forum run ls
git forum run show <RUN_ID>
```

### Verification

```bash
git forum verify <THREAD_ID>
git forum policy lint
git forum policy check <THREAD_ID> --transition <TRANSITION>
```

## 13. Command behavior requirements

### 13.1 `git forum init|doctor|reindex`

- `.forum/` ディレクトリを作る。
- default `policy.toml` を生成する。
- default templates を生成する。
- `.git/forum/` を初期化する。
- `doctor` は policy / template / local index / ref namespace の整合を検査する。
- `reindex` は Git refs から local index を完全再構築する。

### 13.2 `git forum issue|rfc|decision new`

- create event を作る。
- `--body` または `--body-file` で初期 thread body を与えられる。
- thread id を採番する。
- initial state を設定する。
- 対応する ref を作成する。

初期 state:

- issue: `open`
- rfc: `draft`
- decision: `proposed`

### 13.3 `git forum say`

- 指定 node type が policy で許可されているか検証する。
- AI actor の場合は provenance を要求する。
- say event を append する。

### 13.4 `git forum revise|retract|resolve|reopen`

- `revise` は対象 node を参照する `edit` event を append する。
- `retract` は対象 node を参照する `retract` event を append する。
- `resolve` / `reopen` は `objection` または `action` にのみ適用できる。
- `resolve` 済み node は open objection / open action 集計から除外する。
- `NODE_ID` は canonical node OID の full ID を受け付けること。
- exact match がない場合は、同一 thread 内で一意な prefix を受け付けてよい。
- prefix 解決の最小長は 8 文字とする。
- prefix が曖昧な場合は候補 full ID 一覧を表示して失敗する。

### 13.5 `git forum show`

最低限、次を表示する。

1. title / body / kind / state
2. labels / assignees / scope
3. open objections
4. open actions
5. latest summary
6. timeline

### 13.6 `git forum node show`

- node id から単一 node を引けること。
- canonical node ID は、その node を導入した `say` event commit の Git OID とする。
- full ID に加えて、global に一意な prefix を受け付けてよい。
- prefix 解決の最小長は 8 文字とする。
- prefix が曖昧な場合は候補 full ID 一覧を表示して失敗する。
- 現在の body と状態を表示すること。
- その node に関係する `say` / `edit` / `resolve` / `retract` / `reopen` の履歴を表示すること。

### 13.7 `git forum state`

- 現在 state から遷移可能か検証する。
- guard を評価する。
- approval が必要なら `--sign <ACTOR_ID>` で渡された actor から `approvals[]` を構成する。
- state event を append する。

### 13.8 `git forum verify`

MVP では以下をチェックする。

- policy violation がないか
- required summary があるか
- open objection が残っていないか
- required evidence が満たされているか

### 13.9 `git forum evidence add|link`

- `evidence add` は対象 thread に evidence object を追加する。
- `link` は thread 間、thread と evidence 間、または decision と issue 間の relation を記録する。
- relation は timeline と detail view から辿れること。

### 13.10 `git forum run spawn|ls|show`

- `run spawn` は run record を生成し、対象 actor / thread / context を紐づける。
- run が thread に書き込む場合は policy と provenance 要件を満たさなければならない。
- `run show` は model / context_refs / tool_calls / result を表示する。

### 13.11 `git forum policy lint|check`

- `policy lint` は `.forum/policy.toml` の構文と参照整合を検証する。
- `policy check` は指定 transition に対して guard 評価結果を dry-run で返す。

### 13.12 `git forum tui`

MVP の TUI は read-first とし、最低限次を提供する。

1. thread 一覧の表示
2. kind / state による基本フィルタ
3. thread detail の表示
4. open objections / latest summary / timeline の表示
5. index 再読込または refresh

thread 作成や state change などの編集操作は、MVP では CLI に委譲してよい。

## 14. ID scheme

MVP では thread と event / node で役割の異なる ID を使う。

### 14.1 Thread display IDs

thread の primary display ID は人間可読とする。

- issue: `ISSUE-0001`
- rfc: `RFC-0001`
- decision: `DEC-0001`
- run: `RUN-0001`

これらは CLI の一覧・作成・参照に使う display ID である。

### 14.2 Canonical event IDs

- event の canonical ID は、その event を保存した commit の Git OID とする。
- `event.json` は self-referential になるため、自身の canonical OID を authoritative field として重複保持しなくてよい。
- reader / replay / show は、event を含む commit OID を使って canonical event ID を materialize すること。

### 14.3 Canonical node IDs

- node の canonical ID は、その node を導入した `say` event commit の Git OID とする。
- `say` event は canonical node ID を generated opaque ID として別途発行してはならない。
- `edit`, `retract`, `resolve`, `reopen` は canonical node OID を `target_node_id` として参照する。

### 14.4 CLI short-ID resolution

- CLI は full canonical OID を受け付けること。
- exact match がある場合はそれを優先すること。
- exact match がない場合は、8 文字以上の unique prefix を受け付けてよい。
- `node show` は repository 全体で解決すること。
- thread-scoped な node 操作は、その thread 内だけで解決すること。
- prefix が曖昧な場合は候補 full ID 一覧を返して失敗すること。

## 15. Semantic merge

MVP では最小限の semantic merge を実装する。

### Auto-merge

- 新規 `say` event の追加同士
- evidence 集合の追加
- summary の追加

### Conflict

- 同一 thread に対する競合する `accepted` decision
- 同一 thread に対する競合する terminal state
- 同一 `objection` / `action` に対する concurrent `resolve` / `reopen`

conflict 時は synthetic merge event を生成し、`git forum show` で unresolved conflict として表示する。

## 16. AI integration requirements

MVP の AI integration は provider 抽象化を持つが、最初から複数 provider を完全実装する必要はない。

必須要件:

1. actor と run を分離すること。
2. run は provenance 情報を保持すること。
3. AI が thread に対して書けるのは policy で制御されること。
4. AI が state を直接変えられる場合でも human approval を guard で要求できること。

## 17. Search requirements

MVP 検索は lexical 検索でよい。

最低限:

- title 検索
- body 検索
- label フィルタ
- kind フィルタ
- state フィルタ
- assignee フィルタ

高度な semantic search はスコープ外。

## 18. Import / export

MVP では full sync ではなく、最低限の import / export に限定する。

### Import

- GitHub issue -> `issue`
- GitHub discussion / markdown RFC -> 手動補助付き `rfc`

### Export

- `issue` を GitHub issue 形式へ
- `rfc` を markdown へ
- `decision` を ADR 形式 markdown へ

## 19. Error handling

CLI は失敗時に以下を返す。

- 明確なエラー理由
- どの policy / guard / state machine に違反したか
- 再実行のためのヒント

例:

- `transition under-review -> accepted denied: unresolved objections remain`
- `ai/reviewer cannot emit node type claim under current policy`
- `provenance required for ai/summarizer`

## 20. Testing strategy

MVP のテスト環境は Rust の標準的なローカル test workflow を前提とし、外部サービスや常時起動の test server を必須としない。

### Required test layers

1. unit tests:
   replay, state machine, policy, guard, merge 判定、index、search を pure Rust で検証する。
2. integration tests:
   一時 Git repository を作成し、CLI を end-to-end で検証する。
3. TUI tests:
   read-first TUI の表示状態と描画結果を検証する。完全な対話自動化は MVP の必須要件ではない。

### Test isolation requirements

- 各 integration test は独立した temporary directory を使うこと。
- 各 integration test は `git init` された repo を自前で作ること。
- Git の global/system config に依存しないよう、必要に応じて `HOME`, `XDG_CONFIG_HOME`, `GIT_CONFIG_NOSYSTEM=1` を隔離すること。
- test repo では `user.name` と `user.email` を明示設定すること。
- ネットワークアクセスは必須にしないこと。AI integration は mock / fake provider で検証できなければならない。

### Determinism requirements

- clock は差し替え可能であること。
- ID generator は差し替え可能であること。
- 必要なら `GIT_AUTHOR_DATE` と `GIT_COMMITTER_DATE` を固定できること。
- snapshot 的な比較を行う場合、commit hash や timestamp のような不安定値に依存してはならない。

### Recommended verification surface

- `git forum show`
- `git forum verify`
- import / export 出力
- TUI の一覧 / 詳細レンダリング

### CI baseline

CI の最低ラインは以下とする。

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test
```

## 21. Acceptance criteria

MVP 完了条件は次とする。

1. 空の Git repository で `git forum init` が動く。
2. issue / rfc / decision を作成できる。
3. 型付き発言を追加できる。
4. AI run provenance を保存できる。
5. policy による state transition 検証が動く。
6. evidence を追加できる。
7. `git forum show` で open objections / latest summary / timeline を表示できる。
8. `git forum verify` で最低限の guard を評価できる。
9. `git forum reindex` で index を再構築できる。
10. branch で分岐した thread を最小限 merge できる。
11. `git forum tui` で一覧・詳細・基本フィルタを操作できる。
12. Rust stable toolchain で build / test できる。

## 22. Recommended implementation order

1. repository init
2. thread create / load
3. event append / replay
4. `show` renderer
5. `say` / `revise` / `resolve` / `state`
6. policy validator
7. evidence and links
8. AI run / provenance
9. SQLite index
10. simple TUI
11. semantic merge

## 23. Open questions after MVP

- `git notes` をどこまで併用するか
- node revision の UI をどうするか
- merge conflict 解消 UX をどうするか
- TUI の編集機能をどこまで広げるか
- cryptographic signing を GPG / SSH / Sigstore のどれに寄せるか
- external tool call provenance をどこまで標準化するか
