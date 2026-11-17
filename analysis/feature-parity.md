# Classroom native client protocol evidence

The signed Android Classroom 3.70.1008934421 ARM64 snapshot supplies the wire field evidence in `proto/classwork_evidence.proto`. Existing `homeroom.proto` names and `QueryCourse` behavior are independent of this partial schema. The prior one-shot Google Workspace test proved `QueryCourse` only. No additional live calls were made for this evidence.

## Course stream and classwork

| Evidence | What it proves |
|---|---|
| StreamItemQueryRequest builder `0x1986508`, StreamItemQueryResponse builder `0x1986658` | Request mask tag 2, repeated filter tag 3, repeated result item tag 2. This matches the existing opaque wrappers and permits separate compilation. |
| StreamFilter `0x1986038` | Repeated item status tag 1, course ID message tag 2, stream type tag 3, publication status tag 4, topic ID tag 12. The code loads `ACTIVE=2` from pool `0x16240` and `PUBLISHED=2` from `0x31ed8`. A candidate per-course query specifies a nonzero known course ID and the active/published filters. |
| StreamItem2 `0x19828d8` and enum pool `0x31d50` | Type tag 1, assignment tag 2, post tag 3, question tag 4, supplement tag 6. Numeric type values are assignment 2, post 3, short answer 4, question 5 and supplement 7. Decode from the nested child that is actually present and allow unknown future types. |
| Assignment2 `0x1982b94`, Post2 `0x1982ddc`, Question `0x1982ebc`, Supplement `0x1983030` | All four contain StreamItemInfo at tag 1. Assignment and question contain TaskInfo at tag 2. |
| StreamItemInfo `0x198321c`, StreamItemId `0x19789e4`, TaskInfo `0x1983cfc` | Structured course/item ID, creation and edit timestamp, title, description, materials, publication data, topic IDs, due date and grading denominator. The field types and tags are in the proto. Grade denominator uses the builder at `0x19704e8` which defaults its field type to Dart protobuf `DOUBLE_BIT=0x80`. |
| StreamItemReadMask `0x198432c`, StreamItemInfoReadMask `0x1984c68`, TaskInfoReadMask `0x1985094` | Nested mask fields for title, description, publication/sort date, attachments, topic ID, due date and maximum points. See the proto for which masks can be constructed. |
| Material `0x1978268`, MaterialReadMask `0x19820e0` | Attachment name, reference, MIME type, optional open URL and corresponding read-mask fields. Open only deliberately selected safe URL schemes from any future data. Fetching Drive file contents or mutating attachments requires separate investigation. |
| TopicQueryRequest `0x198844c`, TopicQueryResponse `0x19885f4`, TopicFilter `0x1988340`, Topic `0x1987ff0` | A separate topic query, filtered by course, can decode topic IDs and display names. Request header and ordering semantics are unresolved. |

The descriptor marks timestamps as raw signed integers. The application contains separate `dueDateMsFromEpoch` and `postDateMsFromEpoch` fields in its analytics model, but the conversion from the wire to that model is not established. Do not guess the timestamp units for deadline or late-work labels. No pagination or ordering contract was established in this pass.

## Archive state, to-do, submissions and People

- Course state is enum tag 21 in Course (`0x1975ca8`–`0x1975cd4`), requested by bool tag 22 in CourseReadMask (`0x197f090`–`0x197f09c`). Pool `0x162e8` resolves values 0 unknown, 1 active, 2 archived, 3 provisioned, 4 declined and 5 abuse. Missing status must remain unknown.
- SubmissionFilterOptions `0x197bf4c` supports own-user, course and item constraints. SubmissionReadMask `0x198129c` and Submission `0x19796e4` expose display status and live points. The schema omits draft grades and the `createIfNeeded` switch. Per-item authorization and what a missing submission means need separate validation.
- Submission display-status pool `0x31230`: 0 unspecified, 1 assigned, 2 missing, 3 turned in, 4 turned in late, 5 graded without turning in, 6 returned without turning in, 7 returned, 8 graded, 9 returned late, 10 graded late, 11 excused. The separate state enum at pool `0x31240` has different numbers and must not be treated as the display-status enum.
- The course read builder encodes repeated student IDs at tag 8 and teacher IDs at tag 22. The matching masks are tags 7 and 23. LookupCourseUser `0x1974574` / `0x1974900` and QueryUser `0x197e458` / `0x197e5dc` provide candidate scoped lookups for role and display name. The schema does not request emails. Course rosters and name lookup need their own authorization and live data validation.

## Comments

The QueryComment request (`0x197e710`) wraps its read mask and filter in tags 2 and 3. The response (`0x197e8c4`) repeats comments at tag 2. CommentReadMask `0x1980dbc` and CommentFilterOptions `0x197bd54` expose comment text, poster ID and selective filtering by known stream item or submission. Comment `0x1979f68` and CommentId `0x1979de4` identify field numbers and parent IDs. Request visibility, audience and private-comment behavior still need explicit validation before exposing this read in the application.

## Boundaries for web parity

The [RPC literal catalogue](rpc-catalogue.txt) includes write operations, but a discovered path is insufficient to determine payload, role, side effects or compatibility. Posting, editing, enrollment, handing in, returning/marking, attachments, notifications, calendars and teacher tools remain unverified. Keep them distinct from working functionality. Production use also needs a separate credential and authorization design beyond the proven one-shot sign-in.

To reproduce an individual field, use the signed `work/phase2/classroom-arm64.so` in the original workspace and disassemble the builder entry with the retained radare2 bundle. Its r2flutter companion resolves `x27` object-pool offsets through `r2flutter -O pp+0xOFFSET`. Compare helper registration calls with `protobuf-builder_info.dart` and `protobuf-field_type.dart` from the prior workspace. The numeric tag sits in `x2` for common scalars and `x0` for message/enum helpers. Helper `0x196e354` registers a singular message, `0x196ebcc` a repeated message, `0x17af524` a singular enum, `0x1970cd0` repeated enum, `0x19362c0` string, `0x19368b0` bool, and `0x196e1d8` int64. The full candidate disassemblies are retained under `work/phase2/builder-*.txt`.

## Course appearance and archive placement

These values come from the course response, so the desktop renderer can choose the artwork and palette associated with each actual course. They supersede any palette derived from the title, class position, or hash. Archive state uses Course tag 21 with the matching request mask tag 22 as documented above. An explicitly archived course should appear under Archived, separate from Home.

| Message | Wire fields |
| --- | --- |
| Course `0x1975a0c` | `course_setting` message 10 |
| CourseReadMask `0x197eecc` | `course_setting_read_mask` message 9 |
| CourseSetting `0x1976ac4` | `course_theme_setting` message 8 |
| CourseSettingReadMask `0x197fefc` | `course_theme_setting_read_mask` message 7 |
| CourseThemeSetting `0x1976e38` | repeated string `color` 2, enum `photo_render_mode` 3, string `secure_photo_url` 6, string `secure_photo_url_gm3` 9, repeated string `color_gm3` 10, bool `use_gm3_stock_theme` 11, string `secure_photo_url_gm3_dark` 12 |
| CourseThemeSettingReadMask `0x1980174` | bool include colour 1, render mode 2, secure photo URL 3, GM3 URL 4, GM3 colour 5, stock indicator 6, and GM3 dark URL 7 |

The colour helper `0x19714b0` passes the unboxed value `0x42` to the repeated-field registration helper `0x196ec70`. Dart protobuf defines `STRING_BIT=0x40` and `REPEATED_BIT=2`. The adjacent repeated enum helper corroborates the unboxed interpretation with `0x202` (enum plus repeated). The render mode pool `0x2fc98` gives 0 DEFAULT_FULL_OPACITY, 1 BLEND_WITH_THEME_COLOR. The Android CourseEntity model stores named `lightColor`, `color`, and `darkColor` values, and GM3 variants. The signed snapshot does not yet establish the order of the palette array or the web renderer’s theme-blending implementation. No `gstatic.com` theme URL literal was found in the carved string references, so the URL should come from the response instead of being guessed. These appearance fields have not been confirmed in a further live request.

## Mutation envelope evidence

The following paths and wire fields were observed statically. The compiled evidence is in `proto/mutation_evidence.proto`, which deliberately has no services or live call implementation. No write, enrollment, notification, credential, or extra network operation was attempted in this analysis.

| Operation family | Confirmed path and wire evidence | Still needed |
| --- | --- | --- |
| Enrollment | `/homeroom.courses.CoursesService/AddUserToCourse` at `0x181d32c`. Request builder `0x198c4c8` provides course ID 1, enrollment code string 2, user ID 3, role 4, read mask 5. Response builder `0x198c684` provides CourseUser at tag 1. | Trace which combinations of course ID, invite, account identity and code the student UI sends. Target only an explicitly approved test course and the signed-in account. |
| Unenrollment | `/homeroom.courses.CoursesService/RemoveUserFromCourse` at `0x17fe190`, with request at `0x198c790`. Fields are course ID 1, user ID 2 and role 3. The add/remove role pool at `0x33880` gives 1 unknown, 2 teacher, 3 student. | Confirm permission, exact inputs, resulting membership and whether course records become inaccessible. |
| Student Stream muting | `/homeroom.courses.CoursesService/UpdateCourseUser` at `0x1796f80`. Mute and unmute write `CourseUser.isMuted` at tag 15 and select `CourseUserUpdateMask.updateIsMuted` at tag 1. `LookupCourseUser` reads this flag with mask tag 13. | A fresh teacher lookup and exact student membership check precede a write. The same scoped read confirms the result. |
| Course notifications | `WriteCourse` sets `Course.currentUserSetting.notificationsMuted` with `CourseUpdateMask.currentUserSettingUpdateMask.updateNotificationsMuted`. See the notification correction below. | Current-user notification reads use `QueryCourse`. The request is checked offline. |
| Submission writes | `/homeroom.dataservice.HomeroomDataService/WriteSubmission` at `0x17c8270`. Request at `0x198b1b0`: header 1, repeated mutation 2, read mask 3. Mutation `0x197df34`: submission ID 1, submission 2, update mask 3, repeated addMaterials 4. `updateCurrentState` is update-mask bool tag 1 at `0x198a15c`. | TURNED_IN=2 and RECLAIMED_BY_STUDENT=5 are response/model state values from pool `0x31240`. That does not validate allowable writes, drive attachment state, role or optimistic concurrency. Test with an explicitly approved isolated assignment before enabling either action. |
| Comment writes | `/homeroom.dataservice.HomeroomDataService/WriteComment` at `0x185b39c`. Request builder `0x198af10`: header 1, repeated CommentMutation 2, read mask 3. CommentMutation `0x197dd94`: id 1, comment 2, update mask 3. `updateComment` is bool tag 1 at `0x1989e88`. | Private-vs-public parent/visibility behavior, actual mutation header per UI action, ownership, permission and teacher notification behavior. Comment status tag 8 uses 1 unknown, 2 active, 3 trashed. The comment read mask does not expose an include-status field. The filter uses repeated IDs (2/3/4) and visibility (5), but no allowed-status field. |
| Stream posts and classwork editing | `/homeroom.dataservice.HomeroomDataService/InsertStreamItem` at `0x18b08d0` and `/homeroom.dataservice.HomeroomDataService/UpdateStreamItem` at `0x17d37cc`. Builders `0x1986b94`, `0x1986f38` register typed items, read/update masks, optional Drive error mode or assignment targeting. Upsert response `0x19872fc` returns repeated per-item result 1, and each result (`0x1987108`) contains status 1 and item 2. | The required fields, student vs teacher policy, rich text, publication, attachments and group targeting still need controlled validation. A successful top-level gRPC response may still carry per-item failures. |
| Course editing | `/homeroom.dataservice.HomeroomDataService/WriteCourse` at `0x1794798`. WriteCourseRequest `0x198a9c0`: header 1, repeated mutation 2, read mask 3. CourseMutation `0x197da40`: course ID 1, course 2, update mask 3, enrollment code 4. | The enrollment-code field does not prove a join operation in WriteCourse. Use the separate membership service as the candidate to investigate. |

WriteRequestHeader builder `0x198a8ac` registers `operation` as enum tag 1. Pool `0x39f60` gives UNKNOWN=1, INSERT=2, UPDATE=3, DELETE=4. This describes the serialized header rather than proving the correct operation and prerequisites for a specific user action.

The class comment composer follows the insertion call site at 0x185c03c–0x185c134. It selects INSERT from pool 0x2a938, sets the Comment payload on CommentMutation, and explicitly selects COURSE (2) from pool 0xde30. Comment.visibility is tag 9 and Visibility.visibilityType is tag 1. The parent is STREAM_ITEM (2) from pool 0x2fd38. No live comment was posted.

The existing-submission transition follows the UPDATE and TURNED_IN path at 0x187b41c–0x187b524 and the RECLAIMED_BY_STUDENT path around 0x187ae54. The desktop client first reads the current user's submission and checks its exact parent, ID, and allowed transition. It sends a single state update with no material changes and requires a matching response before reporting confirmation. Neither submission action was exercised against Classroom. Uploading or editing submission materials still requires recovering its separate flow.

Enrollment implementation uses /homeroom.courses.CoursesService/AddUserToCourse and RemoveUserFromCourse. The signed APK's AddUser builder call at 0x181d508–0x181d5dc populates UserId and role, and supports the branch with enrollment code in the absence of a course ID. The desktop obtains the signed-in user's ID through a scoped QueryUser (verified live with count only). Leaving first checks that the exact current-user ID is in the requested course's student roster and outside the teacher roster. Join and leave were not exercised against Google.

Own submission attachments use SubmissionReadMask.assignment_submission_mask (14), AssignmentSubmissionReadMask.material_mask (1), Submission.assignment_submission (16), and repeated AssignmentSubmission.material (1). The live overview returned eight own attachment records. Material's reference type pool at 0x32848 labels URL as 4 and DRIVE as 2. The submission mutation's addMaterials (4) uses Material, confirmed by its shared constructor pool slot 0x341e8. The outgoing URL attachment action was not tested live.

Local-file upload still needs the two-stage flow. The APK has /upload/drive/v3/files and an upload worker, and Material includes an isNewUpload boolean at tag 8. DriveItemCreate itself creates editor documents (its enum names include Google Docs, Sheets, Slides, Forms, Drawings, and Vids), so it is not the binary file upload endpoint.

Release 0.8 uses the APK's public Drive upload path for bytes and a distinct Classroom WriteSubmission mutation for the resulting Drive material. The attachment payload sets referenceType DRIVE (2), reference to the returned Drive file ID, and isNewUpload=true (Material tag 8). The app rechecks the signed-in student's editable submission before uploading. It sends only the chosen file and requires a matching Classroom response before claiming the material is attached. If Drive confirms creation and Classroom does not confirm attachment, the file remains in the user's Drive and the UI distinguishes those outcomes. No live upload or Classroom write was exercised.

Release 0.9 scopes private feedback to a freshly resolved own SubmissionId. The recovered CommentFilterOptions.visibility uses a repeated Visibility at tag 5 (builder 0x197be54–0x197be74), and the read mask at tag 7 wraps VisibilityReadMask.includeVisibilityType (tag 1). The app requires PRIVATE=1 and the same submission parent in each returned comment. Five live queries succeeded with no feedback records. The separate private composer reuses the Android INSERT branch with Submission parent type 3 and PRIVATE visibility 1. No private comment was sent by verification.

Attachment removal writes only the selected existing Material ID into the student's matching editable Submission record. The SubmissionUpdateMask.assignmentSubmissionMask is tag 7 (builder 0x198a1a8), and AssignmentSubmissionUpdateMask.updateMaterial is tag 1 (builder 0x198a3a0). The signed enum array at pool 0x39440 gives REMOVE=3. The app requests the resulting attachment list and checks that the target is absent and other identified attachments are present. It does not call Drive deletion. No attachment removal was sent by verification.

Release 0.10 loads invitations through QueryCourse using the current-user filter and latestEnrollmentEvent=INVITED. The repeated enum is tag 3 in CourseFilterOptions (builder 0x197ba24) and INVITED is 3 in pool 0x162d8. The request asks for student invite IDs via CourseReadMask tag 15. The app matches the returned Course.invitedStudentUserId tag 16 against the confirmed current user. The live read succeeded without pending invitations. Accept revalidates that membership and supplies the course ID, current user and STUDENT role to AddUserToCourse, matching the Android path at 0x1e835d0–0x1e83654. No acceptance request was sent by verification.

The Home card's decline handler at `0x1e82540` reads `CourseCardComponent` offsets `0x1f` (course ID) and `0x27` (student ID). The constructor at `0x18f6eb0`–`0x18f6ebc` sets those offsets from the two unboxed fields of `_$CourseCardParameters`, read in reverse order at `0x18f6b74`–`0x18f6b88` and passed to the constructor at `0x18f6df0`–`0x18f6e04`. The parameters builder at `0x1ba8888`–`0x1ba891c` explicitly names its first field `userId` (pool `0xe230`) and its second `courseId` (pool `0xb8d8`). The student branch forwards both values via `0x181cbf4` and `0x1e827f0` to `0x1e82874`, which writes the course ID into the nested CourseId and the user ID into the invitation's int64 field. The request is sent to `/homeroom.courses.CoursesService/DeleteCourseInvitation` (pool `0x26a10`). CourseInvitationId registers the nested CourseId at tag 1 and the invitee at tag 2. DeleteCourseInvitationRequest registers CourseInvitationId at tag 1 and the role at tag 2. Pool `0x26a00` gives STUDENT = 2. Disassembly and pool output are retained in `work/phase2/invitation-decline-id-trace.txt` and `work/phase2/invitation-decline-disassembly.txt`.

The desktop client reads the current user and verifies an own pending student invitation before sending. The decline result is confirmed by a second read of the pending invitations. A failed or uncertain response triggers a read refresh. No decline mutation was exercised against live data.

### Account email notifications

The account uses `QueryUser` with the current-user filter and `UserReadMask.userSettingReadMask` tag 3. `User.userSetting` tag 4 carries the master bool at tag 1 and repeated notification categories at tag 8. Each category contains groups at tag 2. Within each group, tag 1 is its numeric identifier, tag 2 is the localized name, and tag 4 is its email bit. The matching masks are in the builders beginning at `0x1980a14`, `0x1980bc8` and `0x1980ce8`. Query and write requests include the locale as tag 4. The desktop currently asks for British English labels.

Android uses a single UPDATE mutation on the signed-in user. The master path sends at `0x179d1bc`, with the master bool and its update bit. The group path sends at `0x179c6ec`. It puts the group identifier and bool into one category containing one group and uses the distinct nested group mask. The response is confirmed by re-reading the signed-in user and matching the changed setting. Retained evidence includes `work/phase2/email-notification-write-trace.txt` and the annotated `builder-UserSetting*`, `builder-UserNotification*`, `builder-UserMutation.txt` and `builder-WriteUserRequest.txt` files. The Google Workspace read returned the master value and all seven group values. Both write shapes were checked offline.

### Posting a plain-text Stream announcement

CourseSetting tag 5 and CourseSettingReadMask tag 4 expose the student policy. Pool `0x2faf0` maps 2 to post and comment, 3 to comments only, and 4 to read-only. Course tag 19 is the repeated muted UserId and CourseReadMask tag 18 requests it. The client reduces this list to a match against the current user and verifies that user's course role separately. The Google Workspace service rejects a combined course-ID and enrolled-list filter with INTERNAL. The client uses the enrolled-list filter for a multi-course read and the routed course-ID filter for a single course.

The posting branch at `0x17d531c`–`0x17d5358` selects POST from pool `0xe100` and places the shared StreamItemInfo into Post. Pool `0x1c4d8` names the arguments passed from the insert path, including description, user ID, publication inputs and item status. The generic constructor writes the plain description using `0x17c4be8`, author using `0x17a4ca0` and publication data via `0x17d7008`. It sends one StreamItem through the InsertStreamItem wrapper at `0x18b07d8`. The client encodes a single active, published POST with the nested class ID and no assigned item ID. It accepts success only when the returned item has a server ID and the expected course, author and text, without a per-item rejection. See `work/phase2/stream-insert-construction.txt`, `stream-post-info-trace.txt` and `stream-insert-fields.txt`. No announcement was published during verification.

### Deleting your own Stream post

The Android handler calls `0x17d35ac` at `0x1e62a08`. It passes the existing item ID and sets TRASHED (3, pool `0x1c968`) at `0x17d3678`. It creates the update mask at `0x17d36b0` and sends UpdateStreamItem at `0x17d36d4`. The POST mask branch at `0x17d4314`–`0x17d433c` uses the Post child of StreamItemUpdateMask. The builders register post mask tag 2, nested info mask tag 1, and updateStatus bool tag 6. The desktop encoder changes only that status. The trace and mask builders are retained under `work/phase2/stream-update-callers.txt`, `stream-delete-update-mask-call.txt` and `builder-StreamItemInfoUpdateMask.txt`.

Before writing, the desktop looks up the exact course and item IDs with LookupStreamItem and verifies the active status and current user's author ID. Request tag 1 holds the repeated item IDs, tag 2 the read mask. The response is repeated result tag 1 with item tag 3. The live read supplied matching IDs, author and active status. The desktop accepts the deletion only with a matching trashed item or a successful result followed by a lookup confirming the post is gone or trashed. No live deletion was attempted.

### Hidden classes

The account-specific visibility enum at pool `0x2fd00` uses VISIBLE = 1 and HIDDEN_BY_USER = 2. QueryCourse reads `Course.currentUserSetting` tag 20 with read mask tag 21. The nested enum and mask bit are tags 8 and 5 respectively. LookupCourseUser returns the same setting at CourseUser tag 17 using mask tag 15. Both read paths returned matching explicit values for all 17 active courses on Google Workspace.

Android's member-specific path builds `UpdateCourseUser` at `0x1e7a038` and sends it at `0x1e7a1e4`. Its outer update mask selects member settings at tag 2, specifically the visibility bool at tag 5. The unhide handler at `0x1e79ef4` passes VISIBLE into the same method. Another Android feature-flag branch uses WriteCourse with the current-user settings mask. The desktop uses the member-specific path with a fresh signed-in student identity and membership lookup, then checks the returned visibility again. Disassembly is kept in `work/phase2/hidden-class-hide-trace.txt` and `hidden-class-wire-trace.txt`. Neither setting was changed live.

### Class card order

The Android reorder handler reads the previous and next class at `0x1e8453c` and `0x1e845bc`. Its request builder sends them as beforeId (tag 2) and afterId (tag 3), with movedCourseId at tag 1 and a course read mask at tag 4. The previous ID is omitted at the beginning and the next ID at the end. The call goes to `/homeroom.dataservice.HomeroomDataService/ReorderCourse`. Its response carries movedCourse at tag 1. Course tag 35 is the opaque bytes sort key and CourseReadMask tag 36 requests it. See `work/phase2/builder-ReorderCourseRequest.txt`, `builder-ReorderCourseResponse.txt` and `class-reorder-callers.txt`.

The desktop requests the sort keys on the normal enrolled-course read. If keys are present for every visible active class it applies their ascending byte order, which matched the order returned for the 15 visible Google Workspace classes. If any key is missing it keeps the returned order. Before changing the order, it computes neighbours from a fresh read of that account's visible active classes. It checks the response ID and queries the enrolled list to verify the full new order before refreshing Home and the sidebar. No live reorder was sent.

### Stream links

The StreamItemInfo builder registers repeated Material at tag 8. The material carries the URL as reference tag 3, the reference type at tag 4 (URL = 4 from pool `0x32848`), and open URL at tag 7. The announcement insert uses those same Material records alongside the post body and requests returned names and open URLs. If the returned published post confirms its identity and body but omits a requested link, the client performs one item lookup with the material mask. It reports the accepted post and any links that still lack confirmation. The wire test checks the nested material and mask, and links remain in the draft when Classroom does not confirm the post. No link was attached live.

The file staging action reuses the resumable Drive uploader used for student submissions. The attachment uses the recovered Material fields, including DRIVE = 2 and `isNewUpload` tag 8. The returned Drive ID stays in the draft and is reused if publishing fails. Explicit Upload and Post actions keep the two operations distinct. The client matches the attachment in readback by its reference or Drive URL. No file was uploaded or posted live.

The native Stream picker uses Drive's read-only files.list request with a file-name filter and pagination. It requests IDs, names and MIME types and retains them in memory while the picker is open. Existing files use the same Classroom Material record with no isNewUpload bit. The picker does not change file sharing or download file contents. The Google Workspace read returned two pages, with 40 and 35 files. Selection and post publication were checked with synthetic data.

The assignment page reuses the picker for the exact editable submission shown under Your work. The request uses the existing WriteSubmission UPDATE envelope with only addMaterials set, targeting a freshly read own submission and its class and assignment IDs. The file ID is used to detect existing attachments before writing. File selection does not upload a copy and does not submit the assignment. The offline wire check covers the nested target and existing-file flag.

### Stream announcement formatting

The StreamItemInfo builder registers RichText at tag 23, and StreamItemInfoReadMask selects the protocol with enum value 2 at tag 20. In RichText, text is tag 1 and repeated inline styles are tag 2. Style carries repeated type values at tag 1 and an int64 start at tag 2. Pool `0x43138` identifies bold = 2, italic = 3 and underline = 4. The decoder converts UTF-16 positions to UTF-8 boundaries, and the encoder converts edited ranges back while rebasing whitespace trimmed from the draft. See `work/phase2/builder-RichText.txt`, `builder-Style.txt` and `builder-StreamItemInfoReadMask.txt`.

The desktop sends the plain text alongside the rich representation and requests the rich form for confirmation. It warns separately when Classroom confirms the post but its formatting differs. The read probe returned 45 styled descriptions across 15 visible Google Workspace classes. The authoring test used synthetic text with a non-BMP character.

RichText tag 3 carries repeated BlockStyle entries. The BlockStyle builder at `0x1970e98` registers the type enum at tag 1 and the UTF-16 paragraph start as int64 tag 2. Pool `0x43120` labels UNORDERED_LIST as 1. The Android conversion at `0x18a08e0` and `0x18a0928` assigns the list type and its start. The desktop encodes an entry for each marked paragraph and validates the start before rendering. Two visible Google Workspace descriptions had bullet markers. No formatted writes were exercised live.

### Deleting your own comments

Android's delete path calls `0x185b178` from `0x1e9cd84`. It chooses DELETE at `0x185b1f8` (pool `0x2b230`, enum value 4), sets only the existing CommentId in CommentMutation at `0x185b280`, and dispatches WriteComment at `0x185b2a4`. The desktop looks up the exact private or class identity with comment ID filter tag 3 and verifies the current user authored the active record. It confirms the deletion by checking the same identity again. The read-only Google Workspace check returned matching visibility and author for a real class comment. Private comment deletion uses the same operation with the own submission identity and was checked offline. See `work/phase2/comment-delete-writer.txt`.

The Android Comment builder exposes a single string for the body and does not show a separate RichText message or read option for it. The web representation of formatted private comments remains unresolved.

### Classroom links

The Android link builder at `0x1877090` selects the assignment, short-answer and multiple-choice segments `a`, `sa` and `mc`. Its decode route matcher registers course and assignment details, including account-prefixed paths. The desktop parser resolves those links using enrolled course IDs and the existing published classwork read. Copying generates the matching web URL for the supported item types. No browser-session or official Classroom API request is needed. The check round-trips IDs from the live Google Workspace course and coursework reads without opening the web URL.

### Class Google Calendar links

The Android CourseReadMask builder sets `includeCalendarUrl` as bool tag 25 at `0x197f0cc`–`0x197f0d8`. The Course builder omits the corresponding response accessor. An isolated read using only the calendar mask identified tag 24 as a UTF-8 Google Calendar URL on Google Workspace. The normal course read now includes that bit. The native Classwork page and class-filtered Calendar offer the returned URL after validating its HTTPS host. Legacy Google Calendar routes under www.google.com/calendar are normalised to calendar.google.com. The URL is never cached with the offline copy.

### Classwork notifications on Stream

CourseSetting tag 13 holds `classworkInStreamDisplayType` and its read mask bool is tag 12. Pool `0x2fb20` confirms collapsed = 1, expanded = 2, and no classwork notifications = 3. The normal enrolled course read requests this setting. The Stream shows a compact title for collapsed classwork, includes descriptions, deadlines and attachment shortcuts for expanded classwork, and filters classwork notifications from the feed when hidden. This filtering is local to the Stream. Google Workspace returned 14 collapsed values and one expanded value. The hidden-state view uses synthetic data.

### Class card teacher names and Mark as done

CourseReadMask requests owner IDs at bool tag 6. Course tag 7 contains repeated UserId messages. The desktop batches those IDs through the existing name-only QueryUser route, takes the first returned nonempty owner name per class and renders it below the class section. All 15 visible Google Workspace classes returned an owner and name. Names and IDs stay out of offline copies.

The Android localization `streamItemDetailsPageMarkAsDone` surfaces at `0x1a2fee8`. The desktop changes the label and confirmation for the existing Turn in transition when the editable submission has no attachments. This preserves the existing check against the signed-in student's current submission. No assignment state was changed live.

## Class Drive folders and student-visible Meet links

The Classroom ARM64 snapshot (SHA-256 `d41b1587dcc22ec0444b98e90017b0bee12ab753e4f05b969819b75053964dfd`) registers `CourseUserReadMask.courseFolderReadMask` at tag 2 near `0x197f624`, `FolderReadMask.includeOpenUrl` at tag 2 near `0x197fe8c`, and `Folder.openUrl` at tag 3 near `0x1976a54`. The Android CourseUser builder has placeholders before tag 5. A read-only LookupCourseUser for the signed-in user's own course memberships established that the server returns the nested Folder in CourseUser tag 3. All 17 active-class records matched their requested identifiers and decoded a Drive link. The later typed request uses the same mask and matches both identifiers in each response before displaying links.

The same snapshot registers `Course.videoCallInfo` at tag 43 near `0x1975f7c` and `CourseReadMask.videoCallInfoReadMask` at tag 44 near `0x197f230`. The nested state and URL are tags 1 and 2 in both masks and responses. The pool array at `pp+0x2fcd0` names `ENABLED_VISIBLE` as numeric 3 and `ENABLED_VISIBLE_TO_TEACHERS` as numeric 2. The Google Workspace class list returned zero Meet links. The positive student-visible case was verified with a synthetic response and requires a live class with an enabled Meet link for full verification. The read-only membership probe also requested `includeStudentAverage` at tag 5, but none of its 17 responses contained a field at tag 6.

The retained analysis uses radare2 disassembly in `work/phase2/shortcuts-disassembly.txt`, the previous annotated Course builders, and the local r2flutter build with `-j -O pp+0x2fcd0`. The live count-only check is `desktop_read_check --class-links`. Neither the diagnostic nor the packaged app persists the Meet or Drive shortcut URLs.

## Question type, choices and the student's own answer

The Android ARM64 builders at `0x1982f88` and `0x198490c` register QuestionInfo and its read mask at tag 3. The nested type is enum 1, with short answer 1 and multiple choice 2 in `pp+0x41070`. Multiple choice is a nested message at tag 3 and its repeated string choices are tag 1 (`0x1984104`). The question read mask requests choices at tag 1 (`0x198529c`). Short-answer editing and peer-visibility rules require additional evidence.

SubmissionReadMask attaches the answer mask at tag 15 (`0x19813d0`), with includeAnswer at tag 1. The Submission answer itself is tag 17 (`0x1979940`). The nested message contains answer type at tag 1, short-answer text at tag 2 and multiple-choice answer text at tag 3. The enum values match the respective names in `pp+0x3ec08`. These are scoped to the current-user submission filter. The recovered write envelope also has a question mask at tag 8 with updateAnswer tag 1. This evidence does not establish the full answer-submission or editing workflow.

The raw instructions and decoded enum arrays are retained under `work/phase2/question-disassembly.txt` and `work/phase2/question-*-pool.json`. Run `desktop_read_check --questions` to verify only aggregate counts against the saved account.

### Android short-answer Turn in

The submit confirmation closure at `0x186f870` enters `0x1ea0c4c`, which calls `0x186f8b8`. That function constructs an existing `SubmissionId` (`0x186f940`), sets Submission tag 6 to TURNED_IN, passes the text into the generated short-answer field through `0x179a514`, then attaches the new QuestionSubmission at tag 17 through `0x1797230`. The allocated type at `0x1ea0d28` resolves to QuestionSubmission (CID `0x14e5`). The outer operation is UPDATE (`0x186f9a0`). It sets both current-state and nested answer updates (`0x186fa74`, `0x186fa94`, `0x186faa4`) before calling the WriteSubmission wrapper at `0x186fad8`. The implementation encodes the text field alone in QuestionSubmission, matches both submission identifiers, and rereads published Classwork and the signed-in student's existing submission before sending. Disassembly is retained in `work/phase2/question-turn-in-branch.txt` and `work/phase2/question-turn-in-helpers.txt`. Test fixtures inspect encoded bytes without sending a write. Multiple-choice, creation of submission records, editing permissions and peer replies require their own verified paths.

### Multiple-choice first-answer composer and editing permission

The native multiple-choice composer uses the shared QuestionSubmission update envelope with the recovered answer-type value 2 and multiple-choice answer at tag 3. Those fields and the update mask are confirmed by the Android message builders. This composition is inferred from the shared envelope and confirmed schema. A separate Android multiple-choice send call site and live server acceptance remain unverified. The app validates that the chosen text matches exactly one option in the current published question before attempting a user-initiated write. It reads the signed-in student's existing submission again immediately before write and checks the returned ID, state and exact answer before reporting success.

The teacher-controlled setting `TaskInfo.allowEditAfterTurnIn` is bool tag 5 (`0x1983d9c`), and `TaskInfoReadMask.includeAllowEditAfterTurnIn` is bool tag 5 (`0x1985130`). The app reads and records that flag for questions. Editing an existing answer requires additional workflow evidence and implementation.

### Editing a short answer

The edit path uses the recovered short-answer Turn in envelope with the same existing own SubmissionId. Before sending, it reads the published question and own submission again. An edit requires explicit teacher permission, a turned-in or late turned-in display state, no released grade, and the same submission ID and original answer captured when the editor opened. The app leaves Google’s final eligibility enforcement in place. Local fixtures exercise these conditions and draft preservation. The actual write path was not exercised against Google.

### Student assignment rubrics

Android registers `/apps.edu.lms.rubric.api.RubricService/ListRubrics` near `0x1839304`. The request uses filter tag 1, parent tag 2 and Assignment id message tag 1. The assignment identifier contains int64 course and item IDs at tags 1 and 2. The response lists rubrics at tag 2. A rubric contains title tag 2, criteria tag 5 and parent tag 6. Criterion fields are ID, title, description and repeated rating at 1 through 4. Rating fields are ID, title, description and points, also at 1 through 4. See `work/phase2/rubric-disassembly.txt` for the ARM64 registrations. The client checks the returned assignment parent before associating any rubric with the open assignment.

Own QuerySubmission requests use liveRubricScoreMask tag 25 with nested criterion mask tag 1. Rating ID and points are the two criterion mask fields. The response carries released scores at tag 26 with map tag 1. The app only exposes those values on returned submissions and matches criterion and rating by ID. The draft score field is outside the read schema.

### Overall grade sharing

Android uses GradebookSettingsLookup, referenced at `0x195012c`. The request has mask tag 1 and repeated CourseId tag 2. The nested display and calculation mask bits are 1 and 2. The response has results at tag 1, and each result has a course ID at tag 2 and settings at tag 3. The settings carry a second course ID at tag 1, grade display enum tag 4 and calculation enum tag 5. From `pp+0x421d8`, student-visible is 1, hidden is 2 and teacher-only is 3. From `pp+0x421c8`, total points is 1 and weighted categories is 2.

The client queries its own CourseUser average only after finding matching settings with student visibility enabled and a recognised calculation method. The CourseUserReadMask average bit is tag 5 (`0x197f668`), and CourseUser.studentAverage is int32 tag 6 (`0x1976770`). The result must match both course and user IDs and have a student role. Android converts the int32 to double and divides by 10,000 at `0x17fed2c`, then writes the value into the course-user model at `0x17fef98`. The student header formatter at `0x1a36a48` passes that same value next to a percent sign. The division by 100 at `0x1885148` is for grading-scale thresholds. Rust keeps the whole percentage value and displays two decimal places. Reproduce the chain with `python3 work/phase2/trace-gradebook-percentage.py`.

### Home invitations and confirmed enrollment navigation

The normal desktop course read explicitly requests the latest enrollment event ADDED (value 2) to keep pending invitations out of enrolled class cards. The separate invitation request uses INVITED (3). Both values come from the Android Enum array `pp+0x162d8`. The Home invitation read runs alongside the other startup reads and is fenced against responses from an invalidated lookup. The Join action uses the earlier identity-bound acceptance call. A positive enrollment result supplies its own course ID, which the app opens after it appears in the authoritative refreshed course list. Offline fixtures cover the navigation and stale response handling.

### Question answer sharing

The Android AArch64 Dart AOT builders register `TaskInfo.submissionsVisibility` at tag 7 (`0x1983dbc`–`0x1983ddc`) and `TaskInfoReadMask.submissionsVisibilityMask` at tag 7 (`0x1985150`–`0x1985170`). The nested message and mask use tag 1 for the visibility enum and its read flag. The pool at `pp+0x2fad0` contains `UNKNOWN_VISIBILITY_TYPE=0`, `PRIVATE=1` and `COURSE=2`. The question writer at `0x17d5930` selects `COURSE` or `PRIVATE` from the teacher's sharing option. The student question details branch at `0x1a28fdc` checks visibility and selects separate notices for short-answer and multiple-choice questions.

Reproduce the nested field annotations from the workspace root with `python3 work/phase2/annotate-question-sharing.py`. The task field registrations are in `work/phase2/builder-TaskInfo.txt` and `work/phase2/builder-TaskInfoReadMask.txt`. Resolve the enum with `DYLD_LIBRARY_PATH="$PWD/work/phase2/radare2/Payload/usr/local/lib" work/phase2/r2flutter-main/bin/r2flutter -O pp+0x2fad0 work/phase2/classroom-arm64.so`.

`SubmissionSummaryLookup` appears in the `rpcToDataServiceSucceeded` operation-label list at `0x1dd2fa8`. The recovered snapshot lacks a full gRPC path and request/response constructors for that label. `classMultipleChoiceAnswers` at `0x185ec24` is a field of `IndividualGradingDetailsViewModel`; that reference establishes UI state, not a student query. The desktop question visibility read uses the published Classwork query and the existing own-submission read. Live results and peer replies still need a traced, authorized request.

### Own turn-in receipt and history read

Android registers `includeLastTurnedInTimestamp` at SubmissionReadMask tag 21 (`0x1981444`) and `lastTurnedInTimestamp` as int64 at Submission tag 22 (`0x19799e0`). The desktop requests it with the existing current-user filter and displays the returned Unix millisecond timestamp in the local timezone.

The full history flag is SubmissionReadMask tag 19 (`0x198141c`). Submission tag 20 (`0x1979984`) is a repeated message with state history at tag 1 (`0x1978cb0`). The nested state is enum tag 1 (`0x1978fb0`) and its time is int64 tag 2 (`0x1978fdc`). The probe `desktop_read_check --submission-history` scopes that request to the signed-in user and one exact submission key. All six live Google Workspace queries returned the requested submission without any decodable state events. Grade history and actor data are not decoded by the probe. The history reader stays outside the desktop interface until the student-visible contract is verified.

### Create a blank Drive editor file from Your work

Android uses `/homeroom.dataservice.HomeroomDataService/DriveItemCreate` at `0x17b1594`. The request builder at `0x197b268` sets CourseId tag 1, editor enum tag 2, title tag 4 and MaterialReadMask tag 5. The response builder at `0x197b424` registers Material at tag 1. Resolve the editor enum with `DYLD_LIBRARY_PATH="$PWD/work/phase2/radare2/Payload/usr/local/lib" work/phase2/r2flutter-main/bin/r2flutter -O pp+0x39370 work/phase2/classroom-arm64.so`. The enum values are Docs 1, Sheets 2, Slides 3 and Drawings 6.

The desktop matches the four editor options in the web workflow. The RPC creates the Drive file. The desktop then rechecks the exact own submission and uses its previously recovered Drive attachment mutation with the validated file ID. The document opens in the default browser after Classroom confirms the attachment. A confirmed creation with an unconfirmed attachment is reported separately. Fixture checks cover the create request bytes, editor type and response identity, and the exact parent in the attachment mutation. No create or attach mutation was sent live.

### Drive sharing options and personal copies

The Android Material builder sets parentReference (string, tag 10 at `0x197783d8`) and driveItem (message, tag 15 at `0x19778474`). The DriveItem builder sets sharingOption (enum, tag 2 at `0x19777db0`). Pool `pp+0x3ea40` identifies COPY 1, VIEW 2, EDIT 3 and COPY_TO_KEEP 4. The read mask uses includeParentReference tag 9 (`0x19821b4`) and driveItemMask tag 15 (`0x198222c`), whose includeSharingOption flag is tag 2 (`0x1981b58`). The client requests sharing options from published material and parent references from the current student's submissions. A personal template matches exactly one own attachment whose parent reference is the original file ID and whose editor URL points to its distinct returned file ID.

The APK's CreateStreamItemMaterialActivity request registers a nested streamItemMaterialActivity at tag 1 (`0x1987e50`). That activity registers its ID and GeminiU notebook data (`0x1987c1c`, `0x1987c58`). The request evidence does not establish a personal-copy creation route. In particular, the client does not set QuerySubmission createIfNeeded during its personal-copy reads.

### Teacher drafts and scheduled items

The same own-member LookupCourseUser request used for Drive folders also requests courseRole. The exact nested user and course IDs must match the request. The course-role enum at `pp+0x33880` gives TEACHER 2 and STUDENT 3. Missing or duplicate role results never grant teacher access. The teacher query independently rechecks this role, uses QueryStreamItem with one course, active status and publication status DRAFT 3, and requests PublicationInfo through the tag 8 nested mask. Publication status is tag 1, scheduled timestamp is tag 5 and scheduled status is tag 6. Pool `pp+0x31ec8` gives SCHEDULED 1, RETRYING 2, SUCCEEDED 3 and FAILED 4. A result must be a draft for the exact class. Drafts are held in the desktop session only and kept out of the published item snapshot and its offline cache.

### Create and edit assignment drafts

The Android UpdateStreamItem envelope uses repeated stream item at tag 1, update mask at tag 2 and read mask at tag 3. StreamItemUpdateMask.assignmentMask is tag 1 at `0x1985368`. Its streamItemInfoMask is tag 1 at `0x19855c0`. Within StreamItemInfoUpdateMask, updateTitle is tag 1 at `0x1985a64` and updateDescription is tag 2 at `0x1985a78`. The desktop uses these two flags for only the changed fields. Editing an existing draft rechecks the teacher role, exact course and item, draft state, and the original text and formatting.

A new assignment uses the existing InsertStreamItem envelope with type ASSIGNMENT 2, active status and explicit publication mode DRAFT 3. It sends the current teacher as creator and defaults to 100 points. The response must identify the exact course, be an assignment in draft state, return the submitted text, and provide a valid assigned item ID. Both actions check per-item rejection codes. Wire tests use local protobuf fixtures.

### Publish and schedule a saved assignment

StreamItemInfoUpdateMask.publicationInfoMask is tag 4 at `0x1985aac`. Its PublicationInfoUpdateMask builder sets updateStatus tag 1 (`0x198a53c`, pool `pp+0x39640`) and updateScheduledTimestamp tag 3 (`0x198a558`). The StreamItemInfo payload holds PublicationInfo at tag 10. Its status enum is tag 1 and scheduledTimestamp is signed int64 tag 5. The immediate publish mutation supplies PUBLISHED 2 with the status mask. The schedule mutation supplies the future epoch-millisecond time with the timestamp mask on the currently unpublished assignment.

The desktop uses the Mac's timezone to interpret schedule entry and checks the exact draft, current teacher role, and unchanged content and publication state before writing. It expects confirmation for the same class and item. A scheduled acknowledgement includes DRAFT 3, SCHEDULED 1 and the exact requested timestamp. Offline fixtures exercise both request shapes, response identity and timestamp parsing.

### Teacher view of a published assignment or question

The SubmissionReadMask builder registers includeStudentUserId tag 7 (`0x198133c`) and includeDraftGradeNumerator tag 9 (`0x1981364`). Returned studentUserId is a message at Submission tag 8 (`0x197985c`), and draftGradeNumerator is a double at tag 10 (`0x197989c`). The teacher read independently confirms exact current-user teacher membership, sends a single StreamItemId parent in the filter, and accepts only submissions for that course and item, with positive and unambiguous student IDs. Names are resolved only for accepted rows. The separate student read continues to request only the signed-in user's released grade. Teacher results are held on the open item and excluded from the local coursework cache.

### Teacher numeric grades and returning an individual submission

The Android teacher grade-save call at `0x1860384` writes the draft-grade field of Submission and calls WriteSubmission at `0x18604c0`. The teacher return call sets the returned state near `0x186483c`, writes live and draft scores when a grade is supplied at `0x1864a98` and `0x1864ae4`, and calls WriteSubmission at `0x1864d2c`. SubmissionUpdateMask gives current-state tag 1, live-score tag 3 and draft-score tag 4. A draft save sends the draft update only. A graded return sets returned state and copies the saved draft score into both score fields. An ungraded return sets the state alone.

The desktop checks current teacher membership, the published item, the exact student and submission ID, and the student's unchanged scores and submission state. It confirms the expected state and score in the returned match. There is a separate explicit confirmation before returning. The recovered workflow is validated with local protobuf fixtures, not a live grade or return mutation.

### Class-wide teacher gradebook

The Grades tab confirms teacher membership before reading the exact course roster and its published item IDs. QuerySubmission accepts repeated StreamItemId parents at SubmissionFilter tag 2. The gradebook sends batches of up to 12 exact parents and requests student identity, state, and scores. It leaves question answers and material masks out of the grid query. A cell requires both an enrolled student and a matching class and item. A duplicate is omitted to avoid showing an ambiguous result. The gradebook stays in memory, and the app refreshes it after a grade or return action. A cell opens the existing item-level grading view with the corresponding student selected.

### Teacher Stream settings

CourseSetting carries studentStreamPostingPolicy at tag 5 and classworkInStreamDisplayType at tag 13. The Android CourseUpdateMask registers courseSettingUpdateMask at tag 5 (`0x19890cc`). That nested mask registers updateStudentStreamPostingPolicy at tag 4 (`0x1989464`) and updateClassworkInStreamDisplayType at tag 9 (`0x19894c4`). The desktop uses WriteCourse UPDATE for one exact class with only changed fields and their matching flags. It rechecks teacher membership and the current values before writing, then reads both values back from the same active class.

### Student muting and the class notification correction

The Android People handler selects its mute/unmute implementation at `0x1ea6878` and `0x1ea68a0`. The unmute request allocates UpdateCourseUserRequest (CID 0x140b), CourseUser (CID 0x1506), and CourseUserId (CID 0x1508), writes the selected member's isMuted at CourseUser tag 15 and uses CourseUserUpdateMask tag 1. The transport reaches the exact UpdateCourseUser path at `0x1796f80`. The desktop checks the signed-in teacher, the student-role membership, the old flag and the returned flag.

The previous notification implementation used the cohort moderation flag. Android's actual per-class notification action at `0x178f7dc` reaches WriteCourse via `0x178f908`. The notification preference is Course.currentUserSetting tag 20 with nested notificationsMuted tag 6. The optional read mask is CourseReadMask.currentUserSettingReadMask tag 21 with includeNotificationsMuted tag 3. The update mask uses CourseUpdateMask.currentUserSettingUpdateMask tag 15 and nested updateNotificationsMuted tag 3. The two features now emit distinct mutations. Reproduce the moderation trace with `python3 work/phase2/annotate-muting.py` from the workspace root.

### Class details editor

The CourseUpdateMask builder registers updateTitle at tag 1 (`0x1989064`), updateSubtitle at tag 4 (`0x19890ac`), updateDescription at tag 7 (`0x1989128`) and updateRoom at tag 10 (`0x1989144`). A combined Class settings request uses the same WriteCourse mutation as the Stream settings. It includes only changed text fields and their masks, with explicit empty strings for cleared optional fields. A fresh teacher and active class check plus an exact readback cover both class details and Stream choices. The desktop updates the course name and details across views after confirmation.

### Teacher Classwork topic management

The TopicInsert request holds Topic tag 1 and read mask tag 2 (`0x198878c`, `0x19887c0`). TopicUpdate holds Topic tag 1, update mask tag 2 and read mask tag 3 (`0x19888cc`, `0x1988900`, `0x1988934`); the nested updateName flag is tag 1 (`0x19882ac`). TopicDelete request and response each carry TopicId at tag 1 (`0x1988b6c`, `0x1988c98`). TopicId combines the server-assigned string ID and class ID. Creation supplies the class with no existing topic identifier. Reproduce the Android builders with `python3 work/phase2/annotate-topic-mutations.py`.

The desktop rechecks current teacher membership and reads TopicQuery before acting. It verifies the exact topic and old name for rename/delete, then rereads the class to confirm a unique created ID, the renamed ID, or the target's absence. The complete teacher list stays in memory. The published snapshot retains topics that its own posts reference.

### Move Classwork between topics and reorder topics

StreamItemInfoUpdateMask.updateTopicId is the list operation at tag 7 (`0x1985b08`). Its enum pool `pp+0x39440` gives REPLACE_ALL 4. Assignment, Question and Supplement masks are tags 1, 3 and 6. The client preflights the teacher role, exact course/item/type/publication state and existing topic, and reads back the replacement. No topic sends an empty repeated TopicId field under the same replace mask. Offline encoding confirms the request carries the item key and topic and omits title, instructions and publication fields.

The Android ReorderTopic builder at `0x1988d40` registers the current topic at tag 1, successor TopicId at tag 2, opaque successor key bytes at tag 3 and read mask at tag 4. The response builder at `0x1988ee0` returns the moved Topic at tag 1. The app finds the successor in fresh CLASSWORK_ORDER, includes the exact successor key when present and verifies the full returned order. The last-position request omits both successor fields, as observed around `0x17e8d40`–`0x17e8d9c`.

The Android student answer summary reads an item-scoped local database query at `0x18ee280`, reached from StudentAnswerSummaryComponent around `0x1940aa8`. Its view model compares MULTIPLE_CHOICE at `0x1865b7c` and counts matching local submission rows at `0x1865c94`–`0x1865d34`. The snapshot still does not establish the server sync request supplying those student-visible rows.

### Archive or restore a class

The Android CourseUpdateMask builder at `0x19891d8` registers updateCourseState as bool tag 16. The WriteCourse UPDATE carries one course with the desired state, active 1 or archived 2, and only that update flag. The app checks the current user's teacher membership, verifies the prior state with exact-ID QueryCourse, and applies the result to Home, Archived classes, To-do and Calendar after an exact readback. The fixture suite covers both encodings and the local transition. The `desktop_read_check --class-archive-read` diagnostic exercises the exact lookup independently of the mutation.

### Assignment draft due date and points

AssignmentUpdateMask.taskInfoUpdateMask is tag 2. TaskInfoUpdateMask registers updateDueDate at tag 1 and updateGradeDenominator at tag 3. The Android writer converts Dart DateTime microseconds to milliseconds around `0x17e3800` and `0x17d58c4`, omitting dueDate for a null date. The desktop sends a masked omission to clear a deadline and an explicit zero denominator to choose Ungraded. Date-only input uses midnight UTC and timed input uses the Mac's local timezone. Date-only encoding, clearing, and the explicit time-selection flag still need server verification.

Creation sends the draft in one InsertStreamItem request. Editing sends one UpdateStreamItem with independent text and task masks, omitting untouched fields. Exact-ID reads check the draft before the update and confirm its title, instructions, preserved formatting and edited deadline or points afterwards. Offline tests exercise invalid dates and points, existing legacy point values, ungraded drafts and field-level encoding.

### Web workflows still outside the verified release

The [student desktop guide](https://support.google.com/edu/classroom/answer/9582544?co=GENIE.Platform%3DDesktop&hl=en) lists mentions, personal document work, originality reports, overall grades and multi-account use. The [teacher desktop guide](https://support.google.com/edu/classroom/answer/9582854?co=GENIE.Platform%3DDesktop&hl=en) and the [grading and feedback guide](https://support.google.com/edu/classroom/answer/16643267?hl=en) also cover class creation and copying, quiz assignments, rubric editing, comment banks, inline file annotation, practice sets and progress reporting. These workflows need separate evidence from the core class and assignment reads. The Google Workspace test account returned student membership in every class, and all classes restricted the overall grade. Teacher-only paths were exercised with offline fixtures where implemented. Education-leader and admin tools have additional role and entitlement requirements.

### Classwork display and within-topic ordering

StreamItemInfo's ordering key is bytes tag 22, requested through StreamItemInfoReadMask bool tag 19. The live Google Workspace response contained keys for all 148 published items. Of 31 multi-item groups, 17 responses arrived in ascending order and three descended. The order of three consecutive items in a signed-in web topic matched ascending bytes. The desktop sorts when every item in the group has a unique key and uses response order when the keys are incomplete or ambiguous. The cache retains the keys alongside the items.

For teachers, ReorderClasswork sends the exact topic and moved item, plus the preceding item and its opaque key when one exists. A fresh read checks the exact displayed topic and item order before the move. Another read verifies the new order after the RPC. The Android drag handler exits when the destination topic is absent. The desktop exposes move controls in named topic groups. Reproduce the protocol trace with `python3 work/phase2/annotate-classwork-order.py`.

### Create a class

CreateCourse uses the CoursesService route with Course and CourseReadMask at request tags 1 and 2. Dart type arguments at `0x18045c8` identify the direct Course reply. The input carries the current user as owner and the title, plus the supplied section, room and custom subject. The desktop requires the signed-in user's class-creation permission and checks the returned ID, owner and details in a separate read before opening the class. Offline fixtures cover permission matching, encoding and confirmed state.

### Student attachment links and originality reports

A personal copy is eligible to open only when the signed-in student's submission contains one distinct Drive item whose parent reference matches the exact template. A Classroom-supplied URL must resolve to that item's ID. Otherwise the app reads `files.get` metadata for that ID and requires an exact, untrashed reply with a matching Google editor or viewer URL. The app preserves the returned URL, including resource keys. See [Drive file metadata](https://developers.google.com/workspace/drive/api/reference/rest/v3/files) and [Classroom's personal document workflow](https://support.google.com/edu/classroom/answer/9073815).

Android registers Assignment.originalitySettings and its read mask at tag 5. OriginalitySettings.state and the includeState flag are tag 1. At `0x187c928`, the app only attaches the Your work action for ENABLED state 1. The action builds `/c/<course>/a/<item>/dw` at `0x1876ff8` and opens it in the browser at `0x1ea5314`. The Classroom URL encoder `0x17d983c` pads decimal IDs with Z before base64 encoding. The desktop follows this path and also accepts those padded links when pasted. Google hosts the report viewer and Run button on the assignment page. [Classroom originality reports](https://support.google.com/edu/classroom/answer/9335819)

### Practice sets

Google's student instructions say that practice sets open from their assignment. The Android Material reference-type enum at pool `0x32848` lists Drive, URL, Form and newer learning resource types without an explicit practice-set member. The retained snapshot has practice-set navigation and telemetry labels, but those do not establish the published assignment field that identifies a practice set. The native assignment screen offers the traced Classroom assignment link so students can reach interactive web-hosted work. [Work on a practice set](https://support.google.com/edu/classroom/answer/13457451)

### Classroom links

The native link handler opens material and announcement links at `/c/<course>/m/<item>/details` and `/c/<course>/p/<item>/details`. Google documents the material route in its [Classroom coursework link reference](https://developers.google.cn/workspace/classroom/reference/mcp/tools_list/list_course_work_summaries). A published school resource list includes [announcement and topic links](https://files-backend.assets.thrillshare.com/documents/asset/uploaded_file/2353/Tgusd/c0b6405d-242d-482f-a75c-073e8faa3337/distance-learning-plan.pdf). The topic route `/w/<course>/tc/<topic>` opens the Classwork tab with that numeric topic selected and keeps the selection through the subsequent classwork load. All destinations are restricted to the signed-in account's course list or its offline copy.

### Initial personal-copy preparation

Android assignment details call the scoped QuerySubmission builder at `0x194795c`, which passes an exact stream item and student filter to the helper at `0x1940030`. The helper sets `createIfNeeded` at tag 4 around `0x194022c`. The native client only offers preparation for an active student assignment when the exact submission query returns no rows. Confirmation sends one non-retried request and rereads the exact submission and template linkage. An existing submission without its personal document is reported separately. Classroom does not guarantee that the create-if-needed request returns a personal copy, so the result is displayed only after checking the linked Drive reference. Reproduce the static trace with `python3 analysis/reproduce-personal-copy-trace.py <workspace>/work/phase2`.
