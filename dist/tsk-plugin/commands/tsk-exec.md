Run the TSK CLI with the given argument. Supported subcommands:

- `off` — one-key disable: sets `enabled:false`, strips TSK hooks (idempotent). Re-enable with `tsk init`.
- `report` — token savings summary.
- `doctor` — self-check.
- `exec <script> <input>` — run a sandboxed analysis script.

Example: `/tsk-exec off` → runs `tsk off`. Report the command output or any error plainly.