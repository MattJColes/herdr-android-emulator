# One shared Current Device

Every agent and the View act on one shared Current Device, set by `boot`; an agent picks
another with `--device`. This matches herdr-ios-simulator's ADR 0002 and departs from
herdr-web-browser's tab per agent: each Device costs about 2 GB of memory, most people run
one at a time, and a per-agent mapping needs `pane.closed`/`pane.moved` bookkeeping. We
first chose a Current Device per agent and reversed it to keep the two plugins behaving
the same. Per-agent Devices are the upgrade path for both if two agents sharing one becomes
common.
