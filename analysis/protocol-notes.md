# Early protocol and OAuth experiments

These notes describe the initial protocol prototype. See the project README and desktop documentation for the current client.

The Rust client reached Google's private Classroom host over TLS and HTTP/2. An anonymous, empty gRPC request to `/homeroom.dataservice.HomeroomDataService/QueryCourse` returned `Unauthenticated`. Google reported a missing authentication credential. This response establishes the reachability of the host and its gRPC authentication front end. It does not establish that the service dispatched the request, that an independent OAuth client is accepted, or that the scoped request returns courses for Google Workspace.

The crate generates Rust bindings using `prost` and `tonic`. The course request selects title and subtitle via a recovered read mask and sets the recovered current-user filter. The library can send that request with an access token obtained for an independently authorized client. The loopback test verifies the request, standard bearer metadata, status handling, and course decoding with synthetic data. The CLI exposes a live anonymous probe and offline encoding and decoding. It does not accept access tokens on the command line.

## Build and run

From this directory, use a Rust toolchain with Cargo and `protoc` on PATH. This build used Rust 1.99.1 and Cargo 1.99.1 on macOS ARM64.

```sh
cargo test --locked
cargo run --locked -- probe
cargo run --locked -- encode-course-request /tmp/classroom-course-request.pb
xxd -p /tmp/classroom-course-request.pb
```

The expected request bytes are `1204280140011a060a0412020801`. `decode-course-response <file.pb>` accepts a raw protobuf message, without the gRPC frame header. The integration test includes a separately assembled synthetic response fixture. `google-probe.txt` records the live anonymous probe and its expected nonzero exit.

## Authorization check

Google's [published OAuth scope list](https://developers.google.com/identity/protocols/oauth2/scopes) omits `https://www.googleapis.com/auth/homeroom`. Google's public Discovery API also returned HTTP 404 for `classroom-pa/v1`. Those observations establish a lack of public documentation from these sources. They do not prove what Google's OAuth server will return for your independent client.

With the public client ID of your own Google Cloud **Desktop app** OAuth registration, start with a permitted test account:

```sh
cargo run --locked -- scope-check YOUR_DESKTOP_CLIENT_ID.apps.googleusercontent.com
```

The check opens the default browser, requests just the Homeroom scope, and listens on a random loopback port with OAuth state and PKCE. It reports a validated callback outcome. It discards an authorization code without exchanging it or writing it to disk. Google may show an error directly on its page instead of redirecting. In that case, record only the error label and stop the check. An authorization code would justify testing a proper token exchange and inspecting the granted scope before a single real read. An authorization code alone would prove neither the scope grant nor service access. [Google desktop OAuth documentation](https://developers.google.com/identity/protocols/oauth2/native-app)

The signed-out browser Classroom page redirected to Google's sign-in page. That observation does not identify the signed-in web client's data protocol. An existing account session can save a login prompt during independent OAuth, subject to Google and Workspace policy. The session does not give this desktop program its own grant. If Google rejects the private scope for the test client, an authorized service grant or supported integration from Google is needed. If the same client works for a permitted test account and Google Workspace returns `admin_policy_enforced`, take that error and public client ID to the Google Workspace administrator.

## Reproduce the static recovery

Inputs and tools are retained in this conversation's workspace under `work/phase2`. The ARM64 split is from APKPure's public ARM64 variant for version code 300053404. Google's APK verifier validated each split against the Classroom signer, and the ARM64 base APK is byte identical to the original ARM32 bundle's base APK. The distributed ARM64 Flutter engine identifies Dart `3.15.0-77.0.dev`. The ARM64 snapshot has SHA-256 `d41b1587dcc22ec0444b98e90017b0bee12ab753e4f05b969819b75053964dfd` and snapshot hash `27c8345752566aecd8ff78f22d05b194`.

- `analysis/install-snapshot-profile.py` updates the local r2flutter checkout's class IDs using the exact [Dart class table](https://github.com/dart-lang/sdk/blob/3.15.0-77.0.dev/runtime/vm/class_id.h). Rebuild the local tool with `make gen` and `make` against the workspace copy of radare2 6.2.4. Upstream's default profile and 3.14 override could not parse the object pool. Blutter expected separate VM and isolate symbols that this combined snapshot lacks.
- `analysis/annotate-builders.py` disassembles the ARM64 builder functions and resolves object pool offsets with the rebuilt r2flutter. It writes the full annotations under `work/phase2/builder-*.txt`. The checked-in `analysis/annotated-builders.txt` collects the relevant instructions.
- Both architecture snapshots agree on the 35 literal paths in `analysis/rpc-catalogue.txt`. `ListCourses` was recovered as part of a dispatch table at `0x1dd2fa8`, but its full wire path and pagination contract remain unresolved.
- The `QueryCourseRequest` builder starts at `0x197e1dc`. Its nested builder references match `CourseReadMask` and `CourseFilterOptions`. The response builder at `0x197e324` attaches repeated `Course` records. `Course` uses the same `CourseId` closure resolved at `0x1975520`. String, bool, int64, optional message and repeated message registrations were identified by their compiled helpers and compared with [Dart protobuf's field flags](https://github.com/google/protobuf.dart/blob/master/protobuf/lib/src/protobuf/field_type.dart). The schema intentionally omits fields that are not needed for the first read.
- The APK code includes a transport setup near `0x1dd3318` with `classroom-pa.googleapis.com`, `grpc` and port 443, plus an adjacent protobuf content selection. It constructs `Authorization: Bearer` and `X-Goog-AuthUser` in the session code near `0x1e28acc`. The snapshot also contains `x-goog-api-client`, `Accept-Language` and several binary extension headers. Their necessity on a real `QueryCourse` call and the precise route selected for each call remain unverified. Dart groups the Homeroom, Drive and People scopes at `0x166a948` and `0x1f1ac0c`. The Android plugin supplies account and scope inputs to Google Play Services. No first-party client identity or credential is reused in this client.
- Stream and submission RPCs have partial request and response envelopes. Their embedded messages remain raw serialized protobuf bytes. The submission binding omits `createIfNeeded`. The course-list request's acceptance, enrollment filtering and completeness still need an authorized live response. The project has no GUI or account storage yet.

## Android authentication trace and live gateway comparison

The protocol crate, schema, and `scope-check` command are unchanged. Run the separate diagnostic with:

```sh
cargo run --locked --example auth_boundary -- --live
python3 analysis/auth_static.py
```

The diagnostic sends three empty requests using the existing TLS client. The invalid credential is a fixed, intentionally invalid string. It prints gRPC codes, the presence of a Bearer challenge, and a restricted structured error reason. It does not read an account, use a valid credential, or print a response body. The recorded results are in `analysis/auth-boundary.txt`. The nonexistent method returned `Unimplemented` with no challenge, while `QueryCourse` returned `Unauthenticated` with a Bearer challenge. This distinguishes the course route at the gateway. A valid token would be needed to determine the subsequent service or Workspace checks.

### APK evidence

- Dart AOT at `0x166a924`–`0x166a9ec` constructs the SSO scope argument with Homeroom, Drive and People, then references `plugins.flutter.io/sso_auth`. The group also appears at `0x1f1ac0c`. The APK does not show that every scope in the group is necessary for a course read.
- In the retained JADX output, `defpackage/btd.java:352` receives scopes, and `btd.java:485` dispatches `getToken` for the selected account email. `defpackage/pc.java:292` constructs an Android Google account and passes the scopes as `oauth2:` to `bvf`. `defpackage/bvf.java:35` and `bvg.java:25` show the Play Services Auth flow via GAIA ID or matching local Google account. The legacy fallback populates the caller package in `defpackage/buo.java:482`. The app package is Google's signed `com.google.android.apps.classroom`. The traced OAuth branch supplies account and scopes to the broker. Its exact OAuth client ID or server-side client mapping is unresolved. Google's [Android client authentication documentation](https://developers.google.com/android/guides/client-auth) describes certificate binding for certain Play Services flows. This does not establish the token's replay properties.
- Dart AOT constructs `Authorization: Bearer` and `X-Goog-AuthUser` near `0x1e28acc`. One nearby group of code labels a separate binary header as Classroom routing information. The evidence does not establish any binary header as attestation or establish which headers the backend requires. The public-facing probe uses no Android headers.
- The bounded scan in `analysis/auth-static.txt` found no explicit Play Integrity token, SafetyNet, or DPoP marker in the scanned Classroom smali, manifest and carved Dart strings. The DroidGuard match is an auth error constant. The APK also includes FIDO attestation types. Neither identifies a proof attached to the traced course request. These searches do not cover Play Services internals, dynamic flags, or Google server-side checks.
- `defpackage/btd.java:410` has a separate Android AccountManager web continuation helper. That helper runs under the Google app's account context. It does not expose an authorized route for an independent desktop client. The public Classroom web entry point redirected to Google's account sign-in. Its authenticated protocol is still unobserved.

### Authorized next experiment

A stock Android phone or official Play-enabled emulator is a useful control for whether the stock app can open the account and list courses. It does not transfer the official app's authorization. A companion we build would use its own registered package and signing certificate and obtain its own grant through [Google's Android authorization client](https://developers.google.com/android/reference/com/google/android/gms/auth/api/identity/AuthorizationClient). Google requires app registrations for the appropriate [OAuth client platform](https://developers.google.com/identity/protocols/oauth2/policies). Google's [limited-input device flow](https://developers.google.com/identity/protocols/oauth2/limited-input-device#allowedscopes) lists its supported scopes and excludes Homeroom.

Use the preserved `scope-check` with a separately registered desktop client and an authorized test account first. If Google returns a code, exchange it in a future authorized test and check the returned scopes before any Google Workspace sign-in or real course read. A stock-app success on Android answers a different question and cannot replace this test. No browser login, Android session, or real Google Workspace data was used in this pass.
