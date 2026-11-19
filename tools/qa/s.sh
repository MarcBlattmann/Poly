#!/bin/bash
# chrome-qa: send one line of commands to the running driver and wait for its results.
# usage: tools/qa/s.sh TAG 'cmd ; cmd ; ...' [timeout-s]
tag=$1; line=$2; to=${3:-300}
q=target/qa; out=$q/cmds-$tag.out
n=$(awk -F' ; ' '{print NF}' <<<"$line")
before=$(grep -c ' ok \| END' $out 2>/dev/null)
lines=$(wc -l < $out)
echo "$line" >> $q/cmds-$tag
for ((i=0;i<to*5;i++)); do
  now=$(grep -c ' ok \| END' $out 2>/dev/null)
  [ $((now-before)) -ge $n ] && break
  sleep 0.2
done
tail -n +$((lines+1)) $out | grep -v ' ok '
