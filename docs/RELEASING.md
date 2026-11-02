# Releasing Snapshade

Everything here runs from `.github/workflows/release.yml`. A dispatch builds both artifacts for
inspection; pushing a tag `vX.Y.Z` builds them and attaches them to a draft GitHub release.
The version lives in three places that must agree: `Cargo.toml` (`workspace.package.version`),
`src-tauri/tauri.conf.json` (`version`) and `package.json`.

## macOS: Developer ID, notarization, DMG

The macOS build must be signed with a Developer ID and notarized, or Gatekeeper refuses it on
every other Mac. `scripts/release-macos.sh` does the whole sequence in the right order: build
the `.app`, re-sign the nested binaries and the bundle with a **team-scoped designated
requirement** (so the Screen Recording grant survives updates), notarize and staple the `.app`,
build the `.dmg` from the signed app, notarize and staple the `.dmg`. Re-signing after
notarization invalidates the ticket, which is why the order is fixed.

Locally:

```sh
APPLE_SIGNING_IDENTITY="Developer ID Application: <Your Name> (<TEAMID>)" \
SNAPSHADE_TEAM_ID=<TEAMID> NOTARY_PROFILE=<keychain profile> scripts/release-macos.sh
```

The keychain profile comes from `xcrun notarytool store-credentials <name> --apple-id <id>
--team-id <TEAMID>`, so the app-specific password never touches a file. Alternatives the
script accepts: `APPLE_API_KEY` + `APPLE_API_ISSUER` + `APPLE_API_KEY_PATH`, or `APPLE_ID` +
`APPLE_PASSWORD` + `APPLE_TEAM_ID`. Without any of them it skips notarization; with
`APPLE_SIGNING_IDENTITY=-` it leaves the bundler's ad-hoc signature alone (that is what CI does
when no certificate is configured, so a dispatch always produces a `.dmg`).

CI secrets: `APPLE_CERTIFICATE` (base64 `.p12`), `APPLE_CERTIFICATE_PASSWORD`,
`APPLE_SIGNING_IDENTITY`, plus one of the notarization credential sets above. An unset secret
arrives in the job as an **empty** variable, and the Tauri bundler treats a present `APPLE_ID`
as "please notarize", so the script unsets empty variables before building.

Check a release on a second Mac: `spctl --assess --type execute --verbose Snapshade.app` on
a copy that still carries the quarantine attribute should say `Notarized Developer ID`.

## Windows: Azure Artifact Signing

Windows release builds are signed through Azure Artifact Signing (the service formerly called
Trusted Signing). The certificate lives in Azure, is rotated every 72 hours, and every build is
signed with the same publisher identity, which is what SmartScreen reputation attaches to.

One-time setup on the Azure side, in this order:

1. An Artifact Signing account (Basic tier is enough: 5,000 signatures a month; a build uses
   eight: the exe, five NSIS plugin DLLs, the uninstaller and the installer).
2. A **Public Trust** identity validation. For an individual developer the form is populated
   read-only from the Azure billing account's sold-to name and address, which must match the
   government ID you then scan through Microsoft Authenticator's Verified ID with a face check.
   The portal's Identity validations page has an Organization | Individual selector that must be
   switched before clicking **New identity**.
3. One certificate profile of type Public Trust. **Keep it for ever**: a recreated profile has a
   new identity and SmartScreen reputation starts from zero again.
4. An App Registration (service principal) holding the role **Artifact Signing Certificate
   Profile Signer** on the account, with a client secret.

Repository configuration:

| Kind | Name | Meaning |
|---|---|---|
| secret | `AZURE_TENANT_ID` | the tenant of the App Registration |
| secret | `AZURE_CLIENT_ID` | its application id |
| secret | `AZURE_CLIENT_SECRET` | its client secret (note the expiry date) |
| variable | `AZURE_ARTIFACT_SIGNING_ACCOUNT` | the account name; **this is the on-switch**: empty = unsigned builds |
| variable | `AZURE_ARTIFACT_SIGNING_ENDPOINT` | the regional endpoint, e.g. `https://eus.codesigning.azure.net` |
| variable | `AZURE_ARTIFACT_SIGNING_CERTIFICATE_PROFILE` | the profile name |

How a signed build works: Tauri calls `scripts/sign-windows.ps1` (configured as
`bundle.windows.signCommand` in `src-tauri/tauri.windows.conf.json`) for every file it produces,
including the uninstaller from inside NSIS. The hook runs `signtool` with Microsoft's
signing library (`Microsoft.ArtifactSigning.Client`, fetched once from the NuGet CDN with a
pinned version and SHA-256) and a small metadata file naming the endpoint, account and profile;
the library authenticates with the three `AZURE_*` variables, no `az login` involved. The hook
then re-reads the file with `Get-AuthenticodeSignature` and fails the build unless the status is
`Valid`. The workflow fetches the library once before the build and verifies the finished
installer's signature as a separate step.

Two runner facts worth knowing: Windows PowerShell started from a `pwsh` step inherits
PowerShell 7's module path and cannot load its own `Microsoft.PowerShell.Security`, so the hook
pins its module path first; and SmartScreen reputation for a new publisher accrues from real
downloads, so early releases show "Windows protected your PC" even though the signature is
valid.

Local signing uses the same hook: export the three `AZURE_*` secrets and the three
`AZURE_ARTIFACT_SIGNING_*` values in the shell, then `bun run tauri build --bundles nsis`. The
hook downloads the signing library into `%LOCALAPPDATA%\artifact-signing-client` on first use
(`AZURE_ARTIFACT_SIGNING_CLIENT_DIR` points it elsewhere); it needs the .NET 8 runtime, and a
signtool that knows `/dlib` (Windows SDK 10.0.22621 or later, found automatically;
`SIGNTOOL_PATH` overrides). `SNAPSHADE_NO_SIGN=1` builds unsigned.

Check a release: `Get-AuthenticodeSignature Snapshade_<version>_x64-setup.exe` reports `Valid`
and the publisher subject; after a silent install (`/S` from PowerShell) the same holds for
`%LOCALAPPDATA%\Snapshade\snapshade.exe` and `uninstall.exe`.

## Running the workflow locally with nektos/act

act has no Windows runner image, so the Windows job runs in **host mode**
(`-P windows-latest=-self-hosted`): the steps execute on your machine with its own Rust, bun,
`az`, signtool and PowerShell 7, in a per-run copy of the checkout. act refuses host-reaching
flags in a project `.actrc`, so put the mapping, the default Linux image and the artifact/cache
server paths in your user config (`%LOCALAPPDATA%\act\actrc` on Windows), then:

```powershell
scripts\act.ps1 workflow_dispatch -W .github/workflows/release.yml -j windows --var-file .vars --secret-file .secrets
```

`.vars` holds the three `AZURE_ARTIFACT_SIGNING_*` values, `.secrets` the three `AZURE_*`
secrets; both are git-ignored. `scripts/act.ps1` puts Git's bash ahead of WSL's on PATH
(composite actions' `shell: bash` steps break otherwise) and passes every argument through.

Host mode runs actions against your real machine. The workflows skip `Swatinem/rust-cache`
under act (`if: ${{ !env.ACT }}`) because its cleanup step deletes every pre-existing binary in
the real `~/.cargo/bin`, rustup included. Audit any action's post step before adding it.

act does not reproduce the runner image (both signing failures seen so far were runner-only),
cannot run the macOS job, and skips the tag-only release step.
