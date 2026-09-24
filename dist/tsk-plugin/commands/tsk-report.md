Run `tsk report` (no args — it defaults to the current session) with the Bash tool.

Then relay the COMPLETE set of figures in your reply text — never fold, abbreviate, or omit any of them:
- saved (bytes and ≈ est tokens)
- inject cost (bytes and ≈ est tokens)
- net (bytes)
- retrievals (count)
- 压缩原文合计 (bytes and ≈ est tokens) — the combined size of files that were compressed
- 悲观上限 (est tokens for the extra input if the model had read every compressed file in full, and the resulting pessimistic net savings)

Present them as a table or labeled lines. Then add a one-line verdict: if `retrievals` re-read only a small part of the compressed originals, the savings remain large; if the model re-reads close to the full originals, the savings get offset (the per-file full-read cost is the pessimistic upper bound above).