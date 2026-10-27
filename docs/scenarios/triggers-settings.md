# Scenarios: triggers and the settings pages

Each scenario, what should happen, and the test that checks it (or why none can). Test paths are short:

- `model::` is `crates/nekominal-model/src/triggers.rs`
- `term::` is `crates/nekominal-term/tests/triggers.rs`, `term-unit::` is `crates/nekominal-term/src/triggers.rs`
- `shell::` is `crates/nekominal-shell/src/triggers.rs`
- `page::` is `crates/nekominal-shell/src/settings_page.rs`

## 1. Triggers

### Reading the setting

| Scenario | Expected | Test |
|---|---|---|
| Every action kind read from the file, with its defaults | highlight / notify / badge / send / bell read; name defaults to the pattern, notify text to `$0`, highlight colour to the default | `model::a_trigger_is_read_with_its_defaults_and_written_back_the_same` |
| Settings round trip | parse → serialize → parse gives the same rules; `allowSend` written only when true, `enabled` only when false | `model::a_trigger_is_read_with_its_defaults_and_written_back_the_same` |
| Empty pattern, missing pattern | reported as required, rule left out | `model::a_bad_trigger_is_reported_and_left_out` |
| Invalid regex | reported with the parser's last line, rule left out | `model::a_bad_trigger_is_reported_and_left_out` |
| Pattern of exactly 1024 characters, and 1025; counted in characters, not bytes | 1024 taken, 1025 refused; 1024 CJK characters (3072 bytes) taken | `model::the_pattern_cap_counts_characters_and_takes_exactly_the_limit` |
| Huge regex that compiles past the regex crate's size limit (`a{1000}{1000}`) | refused as not a regular expression, not a panic or a stall | `model::a_pattern_past_the_size_limit_of_the_regex_crate_is_refused` |
| Bad action name, missing action, bad colour, send without text, entry not an object | each reported at its path, rule left out | `model::a_bad_trigger_is_reported_and_left_out` |
| Wrong type of an optional key (`"enabled": "yes"`) | reported, the rule kept with the default | `model::a_wrong_type_of_an_optional_key_is_reported_and_the_rule_kept` |
| Short and upper-case colours (`#F00`, `#7A5C00`) | taken | `model::a_highlight_colour_may_be_short_or_upper_case` |
| 64 rules | all read, no problem | `model::past_the_cap_the_triggers_are_left_out_and_that_is_said_once` (67) and `model::exactly_the_cap_is_read_without_a_problem` |
| More than 64 rules | first 64 kept, said once | `model::past_the_cap_the_triggers_are_left_out_and_that_is_said_once` |
| `triggers` not an array | reported | `model::past_the_cap_the_triggers_are_left_out_and_that_is_said_once` |
| Presets are valid and on | each preset compiles and is on | `model::the_presets_are_valid_rules_and_a_send_starts_off_until_its_switch_allows_it` |
| A preset password prompt matches without a line break | instant, matches `Password for x: ` | `model::the_password_preset_matches_a_prompt_that_waits_on_its_line` |

### Send guard and the switch

| Scenario | Expected | Test |
|---|---|---|
| Send is off unless `allowSend` | read as not allowed; `is_on` false | `model::a_send_is_off_unless_the_rule_allows_it` |
| A rule changed into a send starts off; its switch also allows it | `set_action` turns it off, `turn(true)` sets `allowSend` | `model::the_presets_are_valid_rules_and_a_send_starts_off_until_its_switch_allows_it` |
| A send turned off and changed to another action stays off; changing send to send keeps the switch | as said | `model::turning_a_send_off_takes_its_permission_and_changing_it_keeps_the_switch` |
| Per-rule 2 s gap, per-pane 5 a minute | second send of one rule within 2 s refused; sixth in a minute refused | `model::a_send_waits_between_two_of_one_rule_and_a_pane_sends_few_a_minute` |
| The limits at their exact edges | at 2 s exactly allowed; a send exactly 60 s old no longer counts | `model::the_send_limits_hold_up_to_their_exact_edges` |
| A refused send does not count against the limits | a refused try does not push the gap forward | `model::the_send_limits_hold_up_to_their_exact_edges` |
| A send whose text makes the program print its pattern again (a loop) | at most 5 sends a minute however often it matches | `model::a_rule_answering_itself_sends_at_most_five_times_a_minute`, `shell::a_rule_that_matches_what_it_sent_is_held_to_the_pane_limit` |
| The rule that answers itself does not starve another pane | the guard is per pane (`PaneEntry::sends`) | not testable automatically: a `PaneEntry` needs a running pane (ConPTY); the guard is a field of it, which `model::` tests as a unit |

### Notify text

| Scenario | Expected | Test |
|---|---|---|
| `$0`..`$9`, `$$` | match, groups, dollar | `model::the_text_takes_the_match_and_its_groups` |
| Missing group, group that took no part | empty | `model::the_text_takes_the_match_and_its_groups`, `term-unit::the_set_tells_which_rules_match_and_each_hit_has_its_captures` |
| `$10`, `$$1`, `$` at the end, `$a` | `$1` then `0`; `$1` literal; `$`; `$a` | `model::a_dollar_not_followed_by_a_digit_or_a_dollar_stays_as_it_is` |
| `$TAB` (shown in the design mock) | stays literal: the tab's name is not substituted (not implemented) | `model::a_dollar_not_followed_by_a_digit_or_a_dollar_stays_as_it_is` pins it |

### Matching (emulator)

| Scenario | Expected | Test |
|---|---|---|
| A line matched once, when complete | not while written, once after the line break | `term::a_line_is_matched_once_when_it_is_complete` |
| Wrapped line is one line | matched joined | `term::a_wrapped_line_is_one_line` |
| `^` and `$` on a wrapped line | anchor the joined line: `^` does not match the start of the second row, `$` the end of the last | `term::anchors_hold_for_the_whole_wrapped_line_not_each_row` |
| Case sensitivity | case counts unless `(?i)` | `term::case_counts_unless_the_pattern_says_otherwise` |
| Multiple rules match one line | every one fires, in the order of the rules | `term::every_rule_that_matches_a_line_fires_in_the_order_of_the_rules` |
| Overlapping highlights | both kept, drawn in rule order (the later on top) | `term::overlapping_highlights_are_all_kept_in_the_order_of_the_rules` |
| Highlight follows the line into the scrollback | shown again when scrolled back | `term::a_highlight_marks_the_matched_cells_where_the_line_is_shown` |
| Highlight of a line trimmed at the scrollback limit | gone with its line, never on another line | `term::a_highlight_goes_with_its_line_at_the_scrollback_limit` |
| More highlights than kept (1024) | the oldest go first | `term::past_the_most_highlights_kept_the_oldest_go` |
| Resize: rows only | ids kept, highlights stay on their text | `term::a_resize_keeps_the_highlights_unless_the_text_is_reflowed` |
| Resize: width (reflow) | highlights dropped, lines not matched again | `term::a_resize_keeps_the_highlights_unless_the_text_is_reflowed` |
| Clear scrollback, full reset | highlights dropped, nothing matched again | `term::a_cleared_buffer_drops_the_highlights_and_matches_nothing_again` |
| Alternate screen | nothing matched, also instant rules | `term::nothing_on_the_alternate_screen_is_matched`, `term::an_instant_rule_is_quiet_on_the_alternate_screen_too` |
| Instant rule while the line is typed | fires once per line, again on a new line | `term::an_instant_rule_matches_the_line_being_written_once` |
| CR-only redraw (progress bar) | complete rule sees only the final text, once; an instant rule fires on the first text that matches and not again on that line | `term::a_progress_bar_redrawn_with_cr_is_one_line_matched_with_its_last_text` |
| Ink-style app rewrites lines above the cursor | not matched again (known limit, pinned) | `term::a_program_rewriting_lines_above_the_cursor_does_not_fire_them_again` |
| Long line (2048-char cap) | a match inside the first 2048 characters found; text past the cap (plus the rest of that row) not | `term::a_line_is_matched_only_up_to_its_cap` |
| Catastrophic-backtracking-style pattern (`(a+)+$`) on a long line | finishes (the regex crate is linear) | `term::a_pattern_that_backtracks_elsewhere_finishes_on_a_long_line` |
| Pattern that matches empty (`x*`) on a non-highlight rule | fires on every non-empty line with an empty `$0`; blank lines do not fire | `term::a_pattern_that_matches_nothing_fires_on_every_line_but_blank_ones` |
| Program filter: exact, case-insensitive, `.exe` optional, full path | matches | `term-unit::a_rule_with_a_program_matches_only_while_that_program_runs`, `term-unit::the_program_filter_ignores_case_folders_and_exe_on_either_side` |
| Program filter with no job yet (program unknown) | the rule does not match | `term::a_rule_of_one_program_matches_only_while_it_runs` |
| New rules (`set_triggers`) | old highlights go; lines already matched are not matched again | `term::a_highlight_marks_the_matched_cells_where_the_line_is_shown`, `term::new_rules_match_only_new_lines` |
| No triggers | nothing matched | `term::without_triggers_nothing_is_matched` |
| Wide characters and combining marks in a highlight | the spans follow the cells | `term-unit::the_spans_of_a_match_follow_wide_characters_and_wrapped_rows` |

### Actions (shell)

| Scenario | Expected | Test |
|---|---|---|
| Enabled rules become the emulator's rules by index; none → no matcher | as said | `shell::the_enabled_triggers_become_the_emulators_rules_by_index` |
| All rules disabled | no matcher at all | `shell::all_rules_off_is_no_matcher_at_all` |
| Each action | bell, notification (title the name, body substituted), badge (the name), send, highlight does nothing more | `shell::each_action_does_its_own_thing` |
| Send needs `allowSend` and is rate limited | as said | `shell::a_send_needs_the_rule_to_allow_it_and_is_rate_limited` |
| Rule disabled while hits are queued | queued hit does nothing | `shell::each_action_does_its_own_thing` |
| Rule's pattern edited while queued | queued hit does nothing | `shell::each_action_does_its_own_thing` |
| Rule's action edited, pattern kept, while queued | the hit does what the rule does now | `shell::a_queued_hit_does_what_its_rule_does_now` |
| Rule deleted while queued, another with the same pattern takes its index | the hit acts as that rule (pinned: hits are known by index and pattern) | `shell::a_queued_hit_does_what_its_rule_does_now` |
| A rule given its pattern on the settings page has no name | named by its pattern, as the file names it, so its notification has a title and its badge a note | `model::a_rule_without_a_name_is_named_by_its_pattern_when_it_gets_one` (bug fixed) |
| Badge: not when the user watches the tab, not over an exit mark | as said | not testable automatically: `take` needs a `PaneEntry`, which holds a running pane |
| Send writes to the pane | text written | not testable automatically: needs a running pane |

## 2. Settings pages and shared components

| Scenario | Expected | Test |
|---|---|---|
| Every page shows its rows, headings and own entries | counts match | `page::a_section_shows_exactly_its_rows_and_the_shortcuts_page_shows_the_commands` |
| Heading over the row that starts its section, takes no press | as said | `page::a_heading_stands_over_the_row_that_starts_its_section_and_takes_no_presses` |
| Every heading of `settings_ui` HEADINGS is on its page, once | as said | `page::every_heading_is_on_its_page_once_and_never_found_alone` |
| Search finds the push rows by label and note | push url / token found | `page::a_search_finds_the_push_rows_and_never_a_heading_alone` |
| Search never yields a heading as a result; triggers, snippets, layouts are not settings rows | headings only as section names over results | `page::a_search_finds_the_push_rows_and_never_a_heading_alone`, `page::every_heading_is_on_its_page_once_and_never_found_alone` |
| Search for a trigger's name | finds nothing: rules are not rows of the settings table (pinned) | `page::a_search_finds_the_push_rows_and_never_a_heading_alone` |
| Keyboard Up/Down over a heading | the focus skips headings (a heading has no plate to show the focus) | `page::the_keyboard_focus_skips_the_headings` (bug fixed) |
| Enter on a focused row that is a button (snippets, layouts, push test, new profile, a rule) | presses a button that is on that row | `page::enter_on_a_row_presses_a_button_that_is_on_it` (bug fixed: Enter on the snippets and the layouts rows did nothing) |
| Every part has a handler (no dead buttons) | the press handler of the app matches `Part` without a wildcard, and Enter now goes through the same match | compile time: `App::press_page_part` matches every `Part`; the inner handlers' wildcards are not testable automatically (the `App` needs a window) |
| Controls never overlap labels and stay inside rows, narrow and wide, settings rows | as said | `page::at_any_width_a_control_stays_in_its_row_and_clear_of_the_label` |
| Same for the new rows: push test, snippets, layouts, rules (closed and open with sample, error, every action), presets | every pressable item is inside a row, inside the column, and with width | `page::at_any_width_every_part_of_the_new_rows_stays_inside_its_row` |
| Inset from both ends | as said | `page::the_title_labels_controls_and_grids_keep_one_inset_from_both_ends_of_the_rows` |
| `row_label` with a note | the note is under the label, inside the row | `page::a_note_sits_under_its_label_inside_the_row` |
| Push rows, pair on one line, test row after the events | as said | `page::the_phone_notifications_are_under_their_heading_with_every_part_of_the_preview` |
| Masked secret: shown masked, real text only while editing, not in the value read out | as said | `page::the_token_shows_masked_unless_it_is_being_edited` |
| `masked` at its edges (10, 11 characters, CJK) | 10 all dots, 11 shows 3 + 4 | `page::a_secret_shows_its_ends_only_past_ten_characters` |
| Flags: each bit, none, all | each shown chosen on its own; none reads empty | `page::the_events_are_flags_each_shown_chosen_on_its_own`, `page::no_flag_and_every_flag_are_shown_as_they_are` |
| Refused address says why | as said | `page::a_refused_address_says_why_under_its_field` |
| Test row says what the last test came to | as said | `page::the_test_row_says_what_the_last_test_came_to` |
| Triggers page: plates, presets, add button | as said | `page::the_triggers_page_is_a_plate_a_rule_with_its_name_pattern_tag_and_switch_and_the_presets` |
| Open rule: form parts per action | notify/send text field; highlight swatches; badge/bell neither | `page::an_open_rule_shows_its_form_in_place_with_the_sample_matched`, `page::each_action_shows_only_the_parts_it_needs` |
| Live sample with matches; invalid pattern error | as said | `page::an_open_rule_shows_its_form_in_place_with_the_sample_matched`, `page::a_pattern_that_is_not_a_regular_expression_says_so_under_its_field_and_matches_nothing` |
| No sample (no focused pane) | no sample box | `page::each_action_shows_only_the_parts_it_needs` |
| Rule being added | form of its own after the rules | `page::a_rule_being_added_is_a_form_of_its_own_after_the_rules` |
| The rule being added has a name to read out | its plate and its UI Automation name say "Add rule", not nothing | `page::a_rule_without_a_name_is_shown_and_read_out_as_the_rule_to_add` (bug fixed) |
| Snippets, layouts, shortcuts page | as said | `page::the_snippets_are_lines_of_fields_on_the_terminal_page_and_one_being_added_gets_a_line_of_its_own`, `page::the_saved_layouts_are_listed_under_the_profiles_with_a_save_button_and_their_own_buttons`, `page::each_shortcut_of_a_command_is_its_own_button_with_an_add_button_and_reset_all_shows_once_one_changed` |
| A field being edited, then another page chosen | committed as any field is (the press commits first), then the page changes; a refused value is given up with its page | not testable automatically: `App::page_button` needs the window of the settings |
| A refused pattern, then another rule opened or the add button pressed | the refused text is given up with the form it was in (it no longer types into a field that is not shown) | not testable automatically (`App`); fixed in `open_trigger` |
| Escape while editing | the edit is given up, the error with it | not testable automatically (`App::page_key`) |
| Escape on the add form | the field closes; the empty form of the new rule stays open until its plate is pressed (pinned limitation) | not testable automatically (`App`) |
| UI Automation names of the new rows (push test, snippets, layouts, rules, presets) | each row has a name | `page::a_rule_without_a_name_is_shown_and_read_out_as_the_rule_to_add` for the rules; the rest not testable automatically: the names are built in `App::page_access_view` |
