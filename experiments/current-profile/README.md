# Existing-shell startup experiment (not shipped)

This prototype uses the calling PowerShell PID as the only permitted pipe peer. It sends the bootstrap handshake through redirected stdout. Run only in a disposable Windows checkout with an isolated copy of PS7; never dot-source the probe in a user profile.

1. Apply the experimental dispatcher: `git apply experiments/current-profile/service.patch`.
2. Build the service: `cargo build --locked --release --features profile-attach-experiment --example profile_attach_probe`. Use a separately built, unmodified release blueberry.exe as the observer and normal entry.
3. Set BLUEBERRY_NO_HISTORY=1, POWERSHELL_UPDATECHECK=Off, and BLUEBERRY_TEST_PSREADLINE_MODULE to the original 2.4.5 manifest. Run `./experiments/current-profile/measure-attached-profile.ps1 -Destination <new-directory> -Executable <unmodified-blueberry.exe> -Service target/release/examples/profile_attach_probe.exe -Shell (Get-Command pwsh).Source`.

With stock 2.4.5 selected before Profile, the script records the original initialization error in attach-error.txt and stops the disposable shell. This prevents a later installed Profile from supplying another Blueberry process and masking the failure.

To measure the case where the supported private editor is already loaded, point BLUEBERRY_TEST_PSREADLINE_MODULE at its 2.4.5 manifest. Both the native control and the attached candidate use that same module for this diagnostic. The PS7 warm AB/BA results were +501.07/+553.76 ms for the normal entry and -5.76/+66.33 ms for attachment. The two attached samples both produced automatic candidates after the first key.

Run the same command with -CheckUserKey for a separate functional test. It binds g before the service initializes, delegates insertion to SelfInsert, and writes a receipt when the real key handler runs. Both attached runs produced the receipt, first-key echo and automatic static and directory completion. This instrumentation performs file I/O inside the key handler. Exclude it from latency qualification.
