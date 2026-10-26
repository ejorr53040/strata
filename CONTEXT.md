# tesselate

Code intelligence for a repository: an index of symbols and the edges between them, served by a per-repo daemon to a CLI.

## Language

**Daemon**:
The long-lived per-repo process that holds the index in memory and answers queries from the thin client.
_Avoid_: server, service

**Snapshot**:
The index of a repo as built at one moment, with the size and mtime of every file it covers.
_Avoid_: cache, build

**Edge state**:
How confidently a reference was tied to its target, one of the terms below. Every edge carries exactly one.
_Avoid_: confidence, label

### Edge states

**Resolved**:
The reference has exactly one known target in the index.

**Candidate**:
The reference has several possible targets; all are listed and none is guessed.
_Avoid_: ambiguous

**External**:
The target lives outside the indexed repo; the edge names the import as evidence.
_Avoid_: third-party

**Unresolved**:
The reference could not be tied to anything; the edge records why.
_Avoid_: broken, missing

**Unindexed**:
The target lives in code the index skipped, so the absence of an answer is not proof the target is missing.
_Avoid_: ignored

**Inferred**:
Reserved. A target proposed by an LLM rather than by static analysis. Nothing produces it yet.

**Observed**:
Reserved. A target seen at runtime by a tracer rather than by static analysis. Nothing produces it yet.
