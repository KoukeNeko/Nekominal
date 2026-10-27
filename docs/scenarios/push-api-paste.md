# Scenarios: phone push, automation API, image paste and file drop

Each scenario, what should happen, and the test that checks it (`module::test`), or why no test does. "App" means the
scenario runs through `App`, which needs a real window and Direct2D: no test builds one.

## 1. Phone push (`push.rs`, `app/notifications.rs`, `notifications.push.*`)

| # | Scenario | Expected | Test |
|---|----------|----------|------|
| P1 | Empty URL | Nothing is pushed | `push::away_pushes_only_while_the_window_is_in_the_background` |
| P2 | https URL, any host (also unicode, 4000-char path) | Taken | `settings::a_push_url_is_judged_by_its_real_host_whatever_its_spelling` |
| P3 | Plain http to another computer | Refused | `settings::only_https_is_taken_as_a_push_target_and_plain_http_only_to_this_computer` |
| P4 | http to localhost / 127.0.0.1:port / [::1], uppercase scheme or host | Taken | `settings::a_push_url_is_judged_by_its_real_host_whatever_its_spelling` |
| P5 | `localhost@evil`, `localhost:80@evil`, `evil#@localhost`, `evil?@localhost`, `127.0.0.1.nip.io`, `127.0.0.2`, `0.0.0.0`, unclosed `[::1` | Refused: the host is the real one after the user part and before path/query/fragment | same |
| P6 | Leading/trailing space, NBSP, newline in the URL | Refused by the check; the settings field and the file trim outer spaces | same; `settings_ui::the_push_address_refuses_what_it_cannot_send_to_and_takes_empty_as_off`; `settings::a_push_url_in_the_file_the_sender_would_refuse_is_reported_and_left_off` |
| P7 | No host (`https://`, `https://:443/`, `https://user@/x`), other schemes | Refused | `settings::a_push_url_is_judged_by_its_real_host_whatever_its_spelling` |
| P8 | Token never in the log | The log names only the host | `settings::the_log_names_only_the_host_of_an_address`; `push::a_failed_post_says_curls_exit_code_and_not_the_address` |
| P9 | Token and URL never on curl's command line | Sent as a config on stdin; argv is the constant `--config -` | Not testable automatically without reading another process's command line; checked by reading `post` (argv is a literal) |
| P10 | ntfy payload with quotes, CR LF, tab, emoji, CJK, ESC, leading `@`, 140 KB body | Body as is; title one RFC 2047 header line that decodes back | `push::awkward_text_reaches_the_webhook_as_it_was_and_ntfy_in_one_title_header` |
| P11 | Webhook payload with the same text | JSON that parses back to the same strings | same |
| P12 | Config for curl with that text | One line per value, nothing breaks a line | same; `push::the_curl_config_quotes_what_it_is_handed_and_follows_no_redirect` |
| P13 | The request on the wire | Headers (token with quotes, tags, priority) and body byte for byte, through the real curl.exe to a server on 127.0.0.1 | `push::curl_delivers_the_headers_and_the_body_byte_for_byte` |
| P14 | Each event kind → word, choice, ntfy tags, priority | blocked/error/done/command/commandFailed/bell/message each mapped | `push::an_event_has_one_word_for_what_happened`; `push::each_kind_of_event_belongs_to_one_choice_and_has_its_tags_and_priority` |
| P15 | Events filter, each choice alone and none | Only that choice's events; none = nothing | `push::with_one_choice_on_only_its_events_are_pushed`; `push::only_the_kinds_of_event_the_user_chose_are_pushed` |
| P16 | `when` away, window active / inactive; `always` | Away pushes only while inactive; always pushes both | `push::away_pushes_only_while_the_window_is_in_the_background` |
| P17 | Two windows, the other one in front | Away counts the window as inactive and pushes | Not testable automatically (App); per-window `active` is by design |
| P18 | Toast off, push on | Push still sent, through the same limiter | Not testable automatically (App::notify); the limiter itself: `notify::the_limiter_drops_a_flood_from_one_tab_and_caps_bursts` |
| P19 | Flood of events | The limiter drops toast and push alike | same (limiter); the shared call is App |
| P20 | Queue full (target hangs, more than 8 waiting) | Dropped with one log line; a test push says "the queue is full" | Not testable automatically without a curl that hangs for the whole test; logic is one `try_send` |
| P21 | curl fails to connect / HTTP error | Error is `curl.exe exited with 7` / `22`, no URL or token in it | `push::a_failed_post_says_curls_exit_code_and_not_the_address` |
| P22 | curl.exe missing | `curl.exe did not start: …` | Not testable automatically (would need curl removed from System32); one `map_err` |
| P23 | Repeated failures | Logged once until a push gets through | Not testable automatically (log file, global thread); by reading `enqueue` |
| P24 | Test button: refused URL | Sending → Failed(reason the page shows), time kept from the click, page told to redraw once | `push::the_test_button_goes_from_sending_to_what_the_push_came_to` |
| P25 | Test button: delivered | Sending → Sent | same |
| P26 | Test row texts (just now, minutes ago, delivered, failed) | As the design | `settings_page::the_test_row_says_what_the_last_test_came_to` |
| P27 | Settings rows: order, notes, secret token, events flags, field refusal | As the design | `settings_page` and `settings_ui` push tests (existing) |
| P28 | Round trip with every field non-default (quotes, unicode, emoji in token; events subset; empty events) | Read back equal | `settings::every_field_of_the_push_survives_a_round_trip_through_the_file` |
| P29 | Unknown kind / when, bad events list | Default kept, each reported | `settings::the_push_is_off_until_it_has_a_url_and_waits_for_the_user_to_be_away`; `settings::the_pushed_events_are_names_kept_in_their_order_and_a_bad_list_keeps_the_default` |
| P30 | Wrong JSON types for each field; push not an object | Default kept, one problem per key | `settings::a_push_value_of_the_wrong_type_is_reported_and_its_default_kept` |
| P31 | A URL in settings.json the sender refuses | Reported as a problem, left off (**was a bug**: taken silently, every push failed) | `settings::a_push_url_in_the_file_the_sender_would_refuse_is_reported_and_left_off` |

## 2. Automation API (`api.rs`, `app/automation.rs`, `cli.rs`)

| # | Scenario | Expected | Test |
|---|----------|----------|------|
| A1 | Every command with valid args and defaults | Parsed to its request | `api::every_command_parses_from_its_line_with_its_defaults` |
| A2 | Missing / empty / wrong-type members, unknown command, not JSON, not an object | Error names the member | `api::a_request_that_is_not_whole_or_not_right_says_what_is_wrong`; `api::a_member_of_the_wrong_type_or_value_is_named` |
| A3 | Extra members, null members, member given twice, CR LF around, any JSON id | Ignored / missing / last wins / fine / echoed | `api::what_a_request_does_not_need_is_ignored_and_null_is_missing`; `api::the_answer_carries_the_id_of_the_request_whatever_json_it_is` |
| A4 | `lines` 0, negative, float, string, u64::MAX | Errors; MAX is "all" | `api::a_member_of_the_wrong_type_or_value_is_named` |
| A5 | Unknown pane | `no pane …` | Not testable automatically (App); one line in `automation::answer` |
| A6 | Two requests in one write, empty lines between | Both answered in order, empties skipped | `api::lines_are_answered_however_the_writes_cut_them` |
| A7 | Request split across writes, inside a UTF-8 character | Answered once whole | same |
| A8 | Line not UTF-8 | Error answered, client stays (**was a bug**: pipe closed silently) | `api::a_line_that_is_not_utf8_is_answered_and_the_client_stays` |
| A9 | Line over 8 MiB | Error answered, client let go | `api::a_line_longer_than_the_limit_is_refused_and_the_client_let_go` |
| A10 | Partial last line, then the client waits | Server waits for the line break | Covered by A7 (the pause mid-line) |
| A11 | Several / eight concurrent clients | Each answered its own ids | `api::many_clients_at_once_each_get_their_own_answers`; `api::a_client_is_answered_over_a_real_pipe_and_a_subscriber_gets_events` |
| A12 | Subscribe, only the chosen topics, then disconnect | Events of its topics only; dropped at the next event after it goes | `api::a_client_is_answered_over_a_real_pipe_and_a_subscriber_gets_events` |
| A13 | `subscribe` with `events: []` | Subscribes to nothing and waits | Not tested: accepted as is (harmless, the client asked for nothing) |
| A14 | Pipe name already taken | Second server fails | `api::a_client_is_answered_over_a_real_pipe_and_a_subscriber_gets_events` |
| A15 | Security descriptor | Owner the user, protected DACL, one allow entry for the user | `api::the_pipe_is_named_by_the_user_and_the_process_and_open_to_that_user_alone` |
| A16 | Remote clients | Rejected (`PIPE_REJECT_REMOTE_CLIENTS`) | Not testable automatically (needs a second machine) |
| A17 | Discovery: newest live instance, stale file of a crashed one, empty / garbage / path-holding file | Live one found; the others removed; a file a garbage entry names is never written (**was a bug**: a path in it was opened and the request written over it) | `api::the_cli_finds_the_newest_live_instance_and_clears_files_that_name_none` |
| A18 | `--pipe` given a file path | Refused | same |
| A19 | send-text, one line or bracketed paste on | Sent, `sent: true` | Not testable automatically (App); the decision is `logic::paste_warning`, tested in `logic` |
| A20 | send-text multi-line without bracketed paste, or over the size limit | Not sent; window asks; `asked: true` | Not testable automatically (App); decision as A19 |
| A21 | get-text line limits | Last N lines, or all when fewer | `automation::get_text_gives_the_last_lines_asked_for_or_all_there_are`; the pane read is App |
| A22 | close on a busy pane | Asks as the close button does; `asked: true` | Not testable automatically (App) |
| A23 | Window busy (modal loop) | Request retried every 50 ms, answered within 10 s or timed out | Not testable automatically (message-only window and real windows) |
| A24 | CLI argument mapping (options anywhere, `--`, stdin only without words, env pane) | Request as the API takes it | `cli::a_cli_command_becomes_the_request_of_the_api_with_the_pane_it_runs_in`; `cli::a_cli_command_maps_its_options_wherever_they_stand` |
| A25 | CLI bad values | Refused before sending, exit 2 | `cli::a_cli_command_the_api_would_refuse_fails_before_it_is_sent`; `cli::a_cli_command_with_a_bad_value_names_it` |
| A26 | CLI exit codes: ok 0, refused 1, no instance 2, instance gone without answer 2 | As listed | `api::the_cli_exits_0_when_answered_ok_1_when_refused_and_2_when_it_could_not_ask` |
| A27 | `api.enabled = false` | No pipe, no file, no pane variables | Setting: `settings::the_automation_api_is_on_unless_turned_off`; the start is one `.then(api::start)` in `win::run` (App) |

## 3. Image paste and file drop (`paste_files.rs`, `app/drop.rs`, `clipboard.rs`)

| # | Scenario | Expected | Test |
|---|----------|----------|------|
| I1 | Clipboard text only | Text pasted as before | Not testable automatically without the user's clipboard; `App::paste` reads text first |
| I2 | Text and an image both | Text wins | same |
| I3 | Image only, DIB 24 / 32 bit, bottom-up / top-down | PNG saved, path typed | `paste_files::a_bitmap_round_trips_through_png_top_row_first_and_opaque`; `paste_files::a_24_bit_bitmap_pads_its_rows_and_its_alpha_is_kept_when_there_is_one`; `paste_files::an_image_on_the_clipboard_pastes_as_the_path_of_a_png` (ignored: uses the clipboard) |
| I4 | DIBV5 with alpha mask; plain header with masks after it; ALPHABITFIELDS | Alpha kept, masks honoured | `paste_files::a_v5_bitmap_with_masks_keeps_its_alpha_and_a_plain_one_takes_masks_after_the_header` |
| I5 | All alpha zero | Opaque | `paste_files::a_bitmap_round_trips_through_png_top_row_first_and_opaque` |
| I6 | Zero / negative size, 1/8/16 bit, RLE, masks at 24 bit | Refused | `paste_files::a_bitmap_that_claims_more_than_it_holds_is_refused_before_anything_is_allocated` |
| I7 | Truncated header or rows, huge header size, huge colour table | Refused, nothing read past the end | same; `paste_files::a_24_bit_bitmap_pads_its_rows_and_its_alpha_is_kept_when_there_is_one` |
| I8 | Absurd dimensions (i32::MAX², 100000², 10000² with no data) | Refused before any allocation (the pixel cap 2^28 and the length check against the data already exist) | `paste_files::a_bitmap_that_claims_more_than_it_holds_is_refused_before_anything_is_allocated` |
| I9 | A real full-HD bitmap | Read and encoded | `paste_files::a_large_real_bitmap_is_read_and_encoded` |
| I10 | PNG format on the clipboard | Kept byte for byte | Not testable automatically without the user's clipboard |
| I11 | PNG format holding something that is not PNG while a DIB is there too | Falls to reading those bytes as a DIB and fails; the DIB is not tried | Not tested: known limit, rare (a program writing a broken PNG) |
| I12 | Files (CF_HDROP) copied | Their paths typed | Not testable automatically without the user's clipboard; `drop_paths` is shared with the drop |
| I13 | Windows paths with spaces, `&();'`, CJK, UNC, `\\?\` long paths, several files, none | Quoted only when needed, space-separated, empty for none | `paste_files::a_windows_path_is_quoted_only_when_a_shell_would_split_or_read_it`; `paste_files::every_kind_of_windows_path_is_typed_whole` |
| I14 | Windows path with `$` or a backtick into PowerShell | Double quotes let PowerShell expand `$name` | Not tested: known limit (the target does not know cmd from PowerShell; agents' prompts read it fine) |
| I15 | WSL pane: drive letters, `\\?\D:`, `\\wsl$` / `\\wsl.localhost` of its own distro, spaces, CJK, `'`, `$()`, backticks | `/mnt/<d>/…` or the distro path, single-quoted when needed, nothing runs | `paste_files::a_wsl_pane_gets_the_path_under_mnt_in_single_quotes_when_needed`; `paste_files::a_wsl_pane_gets_a_path_its_shell_reads_back_as_it_was` |
| I16 | WSL pane: network share, another distro's `\\wsl$` | Windows path in single quotes (**was a bug**: double quotes, where bash halved `\\` and ran `$(…)`) | `paste_files::a_wsl_pane_gets_the_path_under_mnt_in_single_quotes_when_needed` |
| I17 | WSL pane, which one is WSL | Shell `wsl`; distro from the `wsl-<name>` profile id | Not testable automatically (`target` takes a `PaneEntry`, which needs a running pane) |
| I18 | `paste.images = false` | Nothing typed when the clipboard has no text | Setting: `settings` paste tests; the check is the first line of `App::clipboard_files_text` (App) |
| I19 | Cleanup: older than a day / exactly a day / newer / future | Removed / kept / kept / kept | `paste_files::a_pasted_image_is_cleared_once_it_is_a_day_old_and_not_before`; `paste_files::the_cleanup_removes_only_old_pngs_and_gets_past_what_it_cannot_remove` |
| I20 | Cleanup: missing folder, a held file, a non-PNG, a folder named `.png` | No panic; the rest still removed; others kept | `paste_files::the_cleanup_removes_only_old_pngs_and_gets_past_what_it_cannot_remove` |
| I21 | Drop on a pane / on the tab strip / on an exited pane | Pasted there / nothing / nothing | Not testable automatically (App, `WM_DROPFILES`); `Tab::pane_at` is a rectangle test |
| I22 | Drop several files | All paths, space-separated | `paste_files::every_kind_of_windows_path_is_typed_whole` (the text); the drop itself is App |
| I23 | Broadcast panes | A paste goes to every typed pane with the focused pane's path form; a drop goes only to the pane under it | Not testable automatically (App); a mixed WSL/Windows broadcast group gets one form, a known limit |
| I24 | Drop from an elevated window's lower-integrity Explorer | Blocked by Windows (UIPI) | Not testable automatically; noted in `win.rs` |

## Count

82 scenarios: 31 push, 27 API, 24 paste/drop. 58 are checked by a test (26 of the tests are new); 24 are not, for
the reasons given (mostly that they run through `App`, the user's clipboard, or another machine).
