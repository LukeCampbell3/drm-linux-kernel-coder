# DRM Daily / Weekly Workflow Simulation

Two sequential simulations were executed against the compiled C++ DRM substrate.

- Cold-start week: 81 live Linux task episodes, beginning from an empty DRM state.
- Mature continuation week: 60 task episodes executed without resetting the DRM state learned in week one.
- Capabilities exercised: filesystem, process execution, loopback HTTP, Unix-domain IPC, `/proc`, timers, state commits, notifications, task drift, and ancestral recovery.
- Root semantic vocabulary remained `OBSERVE`, `DERIVE`, `COMMIT`.

Human-time values are scenario assumptions used only to estimate user attention value. Machine timings, success, semantic decisions, vocabulary growth, recovery, and output generation are measured from the live run.
