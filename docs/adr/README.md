# Decision records

An ADR here records a decision that shapes Pigeon and is expensive to undo: the constraint behind it, what was decided, and what it costs. It is not a plan or a roadmap entry; those live in `docs/ROADMAP.md` and the sprint docs.

Format: title, status with the date, context, decision, consequences. Keep each under 30 lines and state what becomes harder and what would make us revisit.

Records are append-only. A changed decision gets a new record that supersedes the old one; the old one keeps its text and gets `Superseded by NNNN` in its status.
