# Project sharing semantics

> **Authoritative spec for sharing behaviour. Any deviation in code is
> a P0 bug.** This document mirrors and extends
> `/Users/bjarne/Code/claude/.scratch/cmem-spec/PROJECT_SHARING.md` —
> if they ever diverge, the spec wins for invariants and this file
> wins for examples.

- [The unit of sharing is "project"](#the-unit-of-sharing-is-project)
- [Three modes](#three-modes)
- [State matrix](#state-matrix)
- [The eight invariants](#the-eight-invariants)
- [Client data flow on `pull`](#client-data-flow-on-pull)
- [Mode downgrades](#mode-downgrades)
- [Revoking a share](#revoking-a-share)
- [Forks](#forks)
- [End-to-end example: Alice + Bob](#end-to-end-example-alice--bob)

---

## The unit of sharing is "project"

cmem-sync does **not** let you share a single observation directly.
You share an entire project; every observation it currently has, plus
every observation pushed in the future, follows the project's share
state.

Why:

- One toggle, one mental model, one row in `project_shares`.
- Past + future observations are automatically covered.
- Permission checks reduce to "does the recipient have an active share
  for this project?".

If you really want to share one finding, see
[Forks](#forks) — you can fork a single observation into another
project and share *that*.

---

## Three modes

| mode | recipient sees | recipient can write | local copy generated |
|------|----------------|---------------------|----------------------|
| `read-only`     | shared_view (in-memory) | no | no |
| `fork-allowed`  | shared_view             | only after explicit `fork` | only the forked rows |
| `auto-copy`     | own_observations        | yes (it's their data) | yes, every pull |

`read-only` is the lowest privilege; `auto-copy` is the highest. A
share's mode is mutable (owner can `PATCH /api/shares/:id`) and the
server tracks downgrades for the recipient (see below).

---

## State matrix

What happens for every (operation, mode) pair on the recipient side:

```
                             read-only     fork-allowed   auto-copy
Bob pulls and sees           shared_view   shared_view    own_obs (auto copy)
Bob queries with claude-mem  yes           yes            yes
Bob can fork explicitly      no (error)    yes -> own_obs already a copy
Bob can edit locally         no            no on shared / yes on forked  yes (his copy)
Bob can re-share to Carol    no            no until forked  yes (it's his data)
Bob multi-machine syncs      each machine each machine    yes (it's now own_obs)
                             pulls fresh   pulls fresh
Owner revokes share          shared_view   shared_view    Bob's own_obs
                             gone          gone (forked   untouched
                                            keeps living)
```

---

## The eight invariants

Any code path that violates one of these is a bug:

1. **Sharing is read-add-only on the owner side.** Receiving never
   mutates the owner's `observations` rows. Recipients write only to
   their own rows.
2. **`shared_view` is ephemeral.** Bob's `read-only` and pre-fork
   `fork-allowed` data lives in `shared_view`, not `observations`.
   Revoking the share evicts the rows from `shared_view`; `observations`
   is untouched.
3. **`auto-copy` is one-way and idempotent.** Each pull copies any
   observation Bob does not already have a `derived_from` row for. No
   double copies.
4. **Forks are write-once.** A fork creates new rows owned by Bob with
   `derived_from = source.id`. Subsequent edits by Alice on the source
   never touch Bob's fork.
5. **Mode downgrades are append-only events.** Server writes to
   `share_mode_downgrades`. Already-forked / auto-copied data **stays
   with the recipient forever**.
6. **Revocation is immediate but non-destructive for derivatives.**
   Bob's `shared_view` clears on the next pull; his forks / auto-copies
   are untouched.
7. **Project ownership cannot transfer.** Sharing is a permission
   grant, not an assignment. The only way to "give" a project is to
   let the recipient fork it.
8. **No cycle in the derivation chain.** `derived_from` always points
   backwards in time; the chain is a DAG, never a cycle. Enforced by
   UUID v7 monotonicity + a server-side check on `fork`.

---

## Client data flow on `pull`

Bob's client maintains two SQLite databases:

```
~/.claude-mem/memory.db                  # claude-mem's own DB (worker writes)
  observations                             # what claude-mem reads back

~/.claude-mem/sync/cmem-sync.db          # cmem-sync's local mirror
  observations                             # Bob's data, pulled from server
                                            (includes auto-copy + fork copies)
  shared_view                              # active shares Bob receives
                                            (read-only / fork-allowed)
  sync_state                               # last_pulled_seq + cursor
```

The two DBs are kept in sync by the worker (claude-mem hooks):
observations the worker writes locally are pushed to the server, and
observations the server returns on `pull` are mirrored into
claude-mem's main DB. **cmem-sync never writes to claude-mem's DB
directly** — it goes through the worker channel to keep the data
contracts clean.

Pseudocode for `pull`:

```rust
async fn pull(&self) -> Result<()> {
    let resp = self.api.pull(self.last_seen_seq()).await?;

    // 1. own observations from other machines
    for obs in resp.own_observations {
        self.local_db.insert_observation(&obs).await?;   // INSERT OR IGNORE
    }

    // 2. shared observations
    for shared in resp.shared_observations {
        match shared.share_mode {
            ShareMode::ReadOnly | ShareMode::ForkAllowed => {
                self.local_db.upsert_shared_view(&shared).await?;
            }
            ShareMode::AutoCopy => {
                if !self.local_db.has_copy_of(&shared.observation.id).await? {
                    let copy = make_copy(&shared, /* derived_from */);
                    self.local_db.insert_observation(&copy).await?;
                    self.pending_push.push(copy);            // upload back next push
                }
                self.local_db.upsert_shared_view(&shared).await?;  // for traceability
            }
        }
    }

    // 3. downgrade notifications
    for down in resp.pending_downgrades {
        self.notify_user(&down)?;
    }
    if !resp.pending_downgrades.is_empty() {
        self.api.ack_downgrades(downgrade_ids).await?;
    }

    self.local_db.set_last_pulled_seq(resp.next_since_seq).await?;
    Ok(())
}
```

---

## Mode downgrades

Going up (`read-only → fork-allowed`, `read-only → auto-copy`,
`fork-allowed → auto-copy`) is silent — nothing breaks for the
recipient.

Going **down** writes a row to `share_mode_downgrades`:

| transition | recipient effect |
|------------|------------------|
| `fork-allowed → read-only` | already-forked rows kept; cannot fork new ones |
| `auto-copy → read-only`    | already-copied rows kept; no new auto-copies |
| `auto-copy → fork-allowed` | already-copied rows kept; new content needs explicit fork |

The recipient's next `pull` returns the row in `pending_downgrades`.
Their client surfaces it as a notification and calls
`POST /api/shared/notifications/ack` after the user acknowledges.

---

## Revoking a share

`DELETE /api/shares/:id`:

- `project_shares.revoked_at = now()`
- recipient's next pull:
  - `shared_view` rows for the project are evicted client-side
  - `revoked_shares` field in pull response carries the project name
  - **forks and auto-copies stay** (they are Bob's data now)

Re-creating a revoked share is fine; nothing forgets the previous
forks. The new share starts fresh.

---

## Forks

Two flavours.

### Fork an entire project

```bash
claude-mem sync fork-project alice/nginx-rce
```

Server steps:

1. Validate share mode allows it (must be `fork-allowed`).
2. Create new `projects` row owned by Bob, `forked_from =
   alice_project.id`.
3. Return list of source observations to copy.
4. Client copies them with new IDs + `derived_from = source.id` and
   pushes back to the server in normal sync.

The recipient ends up with a project that is fully theirs — they can
edit, share, fork, or delete it without touching Alice's original.

### Fork a single observation

```bash
claude-mem sync fork 019d1e20...abcd --to-project my-research
```

Same idea, smaller scope. Useful for cherry-picking findings.
`derivation_chain` is the ordered list of ancestor observation IDs;
forking a fork extends the chain.

---

## End-to-end example: Alice + Bob

```bash
# Alice: set up the project on her two machines
[alice@mac]   $ cd ~/work/nginx-rce && claude-mem sync project init
[alice@mac]   $ claude-mem sync push                 # 50 observations

[alice@linux] $ cd ~/projects/nginx-rce && claude-mem sync pull
              # All 50 observations land in Linux's local DB.

# Alice shares with Bob, fork-allowed
[alice@mac]   $ claude-mem sync share-project nginx-rce --with bob --mode fork-allowed

# Bob pulls
[bob@mac]     $ claude-mem sync pull
              # 50 observations appear in Bob's shared_view.
              # Bob can read them in claude-mem queries but not edit.

# Bob forks one observation he wants to extend
[bob@mac]     $ claude-mem sync fork 019d1e20-abc...0001 --to-project my-research
              # Server creates own_obs row owned by Bob, derived_from = 019...0001
              # Bob can edit / share that fork freely.

# Alice keeps working; new observations stream into the project
[alice@mac]   $ ... edit, push 20 more observations

[bob@mac]     $ claude-mem sync pull
              # 70 total in Bob's shared_view (50 old + 20 new).
              # The forked observation is still in Bob's own_obs and stays Bob's.

# Alice downgrades to read-only
[alice@mac]   $ claude-mem sync share-project nginx-rce --with bob --mode read-only

[bob@mac]     $ claude-mem sync pull
              # Bob sees: "Alice changed nginx-rce share to read-only.
              #          Your existing fork is unaffected."
              # Bob can still read; cannot fork new observations.

# Alice revokes
[alice@mac]   $ claude-mem sync unshare-project nginx-rce --with bob

[bob@mac]     $ claude-mem sync pull
              # Bob's shared_view for nginx-rce is empty.
              # Bob's forked observation is still in own_obs;
              # he still owns my-research with the fork inside.
```

This sequence touches every state transition the spec cares about.
If you can run it end-to-end against your deployment with the
behaviours described above, the share subsystem works.
