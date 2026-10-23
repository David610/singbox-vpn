#!/bin/bash
cd /root/integ
for combo in "new new" "old new" "new old" "old old"; do
  set -- $combo
  pgrep -x postgrest >/dev/null || { (nohup /tmp/pgrst/postgrest pgrst.conf >pgrst.log 2>&1 &); sleep 2; }
  timeout 400 node s4.mjs $1 $2 2>/dev/null > results-$1-$2.json
done
echo done > matrix.done
