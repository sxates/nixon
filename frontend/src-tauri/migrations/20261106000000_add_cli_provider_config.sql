-- specs/0085: the user's "Locate..." override for each subscription CLI, as JSON
-- {"claude-code":{"binaryPath":null|"/..."},"codex-cli":{...}}. Detection results stay in
-- memory; the chosen provider and model reuse settings.provider / settings.model.
ALTER TABLE settings ADD COLUMN cliProviderConfig TEXT;
