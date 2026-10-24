#!/bin/sh
# Scripted log mix for Berth fixture tests (PLAN 8.1): info, warn, a JSON line
# and an error with a 12-line stack trace, repeating every 2 s at LOGGER_RATE=1.
# Higher LOGGER_RATE shortens the pause proportionally.
i=0
while :; do
  i=$((i + 1))
  echo "INFO logger cycle $i started"
  echo "WARN logger cycle $i cache is nearly full"
  echo "{\"level\":\"info\",\"msg\":\"request ok\",\"cycle\":$i,\"ms\":12}"
  echo "ERROR logger cycle $i simulated failure" >&2
  echo "Traceback (most recent call last):" >&2
  n=0
  while [ "$n" -lt 10 ]; do
    n=$((n + 1))
    echo "  File \"/app/worker.py\", line $((40 + n)), in step_$n" >&2
  done
  echo "RuntimeError: simulated failure in cycle $i" >&2
  if [ "${LOGGER_RATE:-1}" -gt 1 ]; then
    sleep "$(awk "BEGIN { printf \"%.4f\", 2 / ${LOGGER_RATE:-1} }")" 2>/dev/null || sleep 0
  else
    sleep 2
  fi
done
