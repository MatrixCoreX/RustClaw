#!/usr/bin/env bash
SKILL_NAME="transform"
DEFAULT_ARGS='{"action":"transform_data","data":[{"user":{"name":"A"},"score":"10"},{"user":{"name":"B"},"score":"20"}],"ops":[{"op":"filter","field":"score","cmp":"gte","value":15},{"op":"project","mappings":[{"from":"user.name","to":"name"},{"from":"score","to":"score"}]}],"output_format":"json","strict":true,"null_policy":"keep"}'
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_run_skill.sh"
