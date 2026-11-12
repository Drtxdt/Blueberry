# Existing-shell startup experiment (not shipped)

This prototype uses the calling PowerShell PID as the only permitted pipe peer. It sends the bootstrap handshake through redirected stdout. Run only in a disposable Windows checkout with an isolated copy of PS7; never dot-source the probe in a user profile.

1. Apply the experimental dispatcher: `git apply experiments/current-profile/service.patch`.
2. Build the service: `cargo build --locked --release --features profile-attach-experiment --example profile_attach_probe`. Use a separately built, unmodified release blueberry.exe as the observer and normal entry.
3. Set BLUEBERRY_NO_HISTORY=1, POWERSHELL_UPDATECHECK=Off, and BLUEBERRY_TEST_PSREADLINE_MODULE to the original 2.4.5 manifest. Run `./experiments/current-profile/measure-attached-profile.ps1 -Destination <new-directory> -Executable <unmodified-blueberry.exe> -Service target/release/examples/profile_attach_probe.exe -Shell (Get-Command pwsh).Source`.

The pilot completed the normal entry but failed to produce an attached sample (Windows error 3). No latency improvement has been established. A separate probe of preloaded stock editors also found no supported first-key hook. Neither partial runs nor a degraded editor qualify as release evidence. Keep diagnostic artifacts private; do not record the handshake token.
