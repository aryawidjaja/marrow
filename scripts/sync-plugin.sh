#!/usr/bin/env bash
# Regenerate plugins/marrow/ from the canonical integrations/claude-code/ sources.
# Run after changing any hook, then commit both. CI checks the two stay in sync.
set -euo pipefail
cd "$(dirname "$0")/.."

cp integrations/claude-code/hooks/*.sh plugins/marrow/hooks/
chmod +x plugins/marrow/hooks/*.sh
cp integrations/claude-code/commands/marrow-save.md plugins/marrow/skills/marrow-save/SKILL.md

python3 - <<'PY'
import json
src = json.load(open('integrations/claude-code/settings.example.json'))
for groups in src['hooks'].values():
    for group in groups:
        for hook in group['hooks']:
            hook['command'] = hook['command'].replace(
                '$CLAUDE_PROJECT_DIR/.claude/hooks', '"${CLAUDE_PLUGIN_ROOT}"/hooks')
with open('plugins/marrow/hooks/hooks.json', 'w') as f:
    json.dump(src, f, indent=2)
    f.write('\n')
PY

echo "plugins/marrow is in sync with integrations/claude-code"
