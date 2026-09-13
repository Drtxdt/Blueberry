# Implementation contract

Windows Terminal launches the Rust executable; Rust starts one pwsh. Runtime contains no Node/JS engine. This independent repository does not modify inshellisense.

Completion offsets are UTF-8 byte offsets. The PowerShell adapter uses UTF-16 offsets; conversion belongs at the host boundary, not in providers or UI. Display columns are a third, separate unit.

Rust model types are defined in src/model.rs. Providers and menu rendering share these types; protocol offset conversion remains centralized in the host.

Private OSC: ESC ] 7776 ; session-token ; JSON BEL. JSON event values: prompt_start, prompt_end (cwd), execute, buffer (line,cursor in UTF-16), commands (commands array). Rust passes BLUEBERRY_TOKEN and BLUEBERRY_EDIT_PATH. Adapter clears the token from child environment after reading it into script state.

PSReadLine bindings: F12,s reports current buffer; F12,a reads a JSON edit from the per-session file, validates expectedLine and expectedCursor, calls Replace(start,length,text), then reports buffer; F12,c reports loaded commands using the current runspace's CommandInvocationIntrinsics.GetCommands API. Command frames carry a UUID snapshot, up to 128 records and a complete flag; only the final page removes old session entries. No additional pwsh processes, no evaluation of returned text. Buffer query is deliberately small; latency is measured before deciding further integration changes.

Normal launch retains user profile once. BLUEBERRY_ACTIVE=1 and legacy ISTERM=1 prevent old inshellisense auto-start recursion in this nested compatibility scenario.
