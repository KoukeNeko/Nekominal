# Scenarios: git worktrees, the Remove this worktree dialog, and the badges of a tab

Each line: the scenario, what is expected, and the test that holds it (module `tests` of the file named in the heading),
or why no automatic test does. Tests that run git make their repositories in the temporary folder and skip themselves
when git is not on the PATH; nothing here uses the network.

## Worktrees: `crates/nekominal-shell/src/worktree.rs`

### Branch names

| Scenario | Expected | Test |
|---|---|---|
| Plain, slashed, unicode, dotted names (`fix`, `fix/login`, `feature/新功能`, `v1.2`, `a@b`) | Valid | `branch_names_follow_the_rules_of_git_check_ref_format` |
| Empty, `HEAD`, `@`, leading `-`, `..`, space, `~^:?*[\`, control characters, DEL, empty or dotted component, `.lock` component, trailing `.`, `@{` | Invalid, before git is asked | `branch_names_follow_the_rules_of_git_check_ref_format` |
| The same names given to `git check-ref-format --branch` | The palette and git agree on every name; `@` is the one exception, which git expands to the current branch and the palette refuses | `branch_names_agree_with_git_check_ref_format` |
| A valid name too long for Windows (260 characters) | git refuses it (`Filename too long`): `Error::Git`, no folder and no branch left behind | `a_branch_name_too_long_for_windows_is_refused_by_git_and_leaves_nothing_behind` |
| A branch that exists already | `Error::Git` with what git said | `a_real_worktree_is_made_beside_the_repository_and_can_be_removed` |

### Where the folder goes

| Scenario | Expected | Test |
|---|---|---|
| `D:\src\app` and `fix/login` | `D:\src\app-fix-login` | `the_worktree_goes_beside_the_repository_named_after_it_and_the_branch` |
| `<>|"` in the branch (git allows them, Windows does not) | Each becomes `-` | same |
| The folder exists, then `-2` exists | `-2`, then `-3` | same, and `a_real_worktree_takes_the_next_free_folder_in_a_path_with_spaces_and_unicode` |
| Repository path with spaces and unicode, unicode branch | Kept as they are | `the_folder_of_a_worktree_keeps_spaces_and_unicode_and_a_drive_root_has_one_too`, `a_real_worktree_takes_the_next_free_folder_in_a_path_with_spaces_and_unicode` |
| Repository at a drive root (`D:\`) | `D:\repo-<branch>` | `the_folder_of_a_worktree_keeps_spaces_and_unicode_and_a_drive_root_has_one_too` |
| Called from a subfolder, or from inside another worktree | The new one goes beside the main checkout | `a_real_worktree_is_made_beside_the_repository_and_can_be_removed` |
| A folder outside any repository | `Error::NotRepo` | same |
| A bare repository | `Error::NotRepo` for create and inspect (there is no checkout to sit beside) | `a_bare_repository_is_not_a_place_for_a_worktree` |
| A submodule | Its own main checkout: inspect says `main`, and its worktrees go beside the submodule's folder | `a_submodule_is_the_main_checkout_of_its_own_repository` |
| A repository with no commits yet | Inspected without counts or default branch; a new worktree is an orphan branch (git 2.42+ infers `--orphan`) or git's refusal | `a_repository_with_no_commits_yet_has_no_counts_and_no_default_branch` |
| A detached HEAD | `branch: None`; not archived (`Refusal::Detached`); the dialog is refused | `a_detached_head_has_no_branch_and_is_not_archived`, `the_main_checkout_and_a_detached_head_are_refused_before_the_dialog` (remove_dialog.rs) |

### What inspect reads

| Scenario | Expected | Test |
|---|---|---|
| Modified, staged, untracked, renamed, conflicted files | Counted as changed (untracked apart), each with its letter (`M`, `A`, `?`, `R`, `U`) | `every_kind_of_change_is_counted_with_its_letter` |
| Only ignored files | Clean | same |
| File names with spaces (git quotes them in `--porcelain`) | Shown without the quotes, also both sides of a rename | `porcelain_paths_lose_the_quotes_git_puts_around_spaces`, `every_kind_of_change_is_counted_with_its_letter` |
| An untracked folder | One entry (`dir/`), as git lists it | `porcelain_counts_changed_and_untracked_files` |
| Upstream set, ahead | `unpushed` = commits ahead | `pushed_ahead_behind_and_unpushed_commits_are_read_against_the_remote` |
| Upstream set, only behind | `unpushed` = 0 | same |
| A remote but no upstream | Commits on no remote branch | same |
| No remote | `remote: None`; every commit counts as unpushed, which the dialog ignores | `a_dirty_worktree_is_refused_until_clean_then_removed_with_its_merged_branch`, `a_repository_with_no_remote_has_nothing_to_push_to` (remove_dialog.rs) |
| A remote without `origin/HEAD` | The default branch falls back to the local `main` or `master` | `pushed_ahead_behind_and_unpushed_commits_are_read_against_the_remote` |
| `origin/HEAD` set | Default branch `origin/main`; merged is checked against it | same |
| Merged vs not merged | `merged` true / false | same, and `a_dirty_worktree_is_refused_until_clean_then_removed_with_its_merged_branch` |
| A branch merged by squash on GitHub | Reads as not merged (git cannot tell); the dialog then preselects keeping the branch, and `git branch -d` keeps it | Not tested: a property of git, not of this code |

### Remove and archive

| Scenario | Expected | Test |
|---|---|---|
| The main checkout | Refused (`Refusal::Main`), also forced | `the_main_checkout_is_never_removed_and_changes_only_by_force`, `a_dirty_worktree_is_refused_until_clean_then_removed_with_its_merged_branch` |
| Changes not committed | Refused unless forced; an archive is never forced | same, and `an_archive_keeps_the_branch_and_refuses_changes` |
| Remove with the branch, branch merged | Folder and branch go | `a_dirty_worktree_is_refused_until_clean_then_removed_with_its_merged_branch` |
| Remove with the branch, branch not merged | Folder goes, `git branch -d` keeps the branch and what git said of it is returned (not the advice git adds after it) | same |
| Archive | Folder goes, branch and its commits stay | `an_archive_keeps_the_branch_and_refuses_changes` |
| A process (a pane's shell) still has the folder as its working directory | git removes the files and the worktree, then fails on the folder; the branch still goes as asked; the empty folder stays while the process holds it | `a_worktree_a_process_still_sits_in_loses_its_branch_as_asked` |
| git fails | The last line git wrote (the `fatal:` line, not the progress before it) | `git_failures_report_the_last_line_git_wrote` |
| git fails in another language | The line is passed on as git wrote it | Not testable automatically: the language of git follows `LANG`/`LC_ALL`, which are process-wide and would change every test running beside it; the extraction is the same as above |
| git is not on the PATH | Every call is `Error::NotRepo` ("not a git repository (or git is not installed)") | Not testable automatically: `PATH` is process-wide, and the tests run in parallel; every git test skips itself instead |

### Panes in the folder (`contains`)

| Scenario | Expected | Test |
|---|---|---|
| The folder itself, a subfolder, other case, forward slashes, trailing separator | Inside | `a_folder_is_in_a_worktree_whatever_its_slashes_and_case`, `a_folder_is_in_a_worktree_only_below_it` |
| A sibling that starts the same (`app` and `app2`, `app-fix` and `app-fix-2`), the parent | Not inside | same |
| A worktree at a drive root, unicode letters in another case | Inside | `a_folder_is_in_a_worktree_only_below_it` |
| A path with `..` in it, or a WSL path (`/mnt/d/...`) for a Windows folder | Not resolved: compared as written | Not tested: shells report their folder resolved, and WSL panes report a Windows path through OSC 7 |

### `.worktreeinclude`

| Scenario | Expected | Test |
|---|---|---|
| Missing | Nothing copied, no error | `a_missing_or_empty_worktreeinclude_copies_nothing` |
| Empty, or only comments | Nothing copied, no error | same |
| Comments, plain names, globs (`secrets/*.key`) | The ignored files they name are copied | `the_ignored_files_worktreeinclude_names_are_copied_into_a_new_worktree` |
| Named but not ignored by git | Not copied (the checkout has it, or it is the user's to add) | same |
| Ignored but not named | Not copied | same |
| Already in the new worktree | Left alone | same |
| Negations (`!`), later lines winning | As `.gitignore` reads them | `worktreeinclude_negations_nested_folders_and_odd_names_are_followed` |
| `**/` patterns and folders several levels deep, a folder pattern (`secrets/`) | Copied with their folders made | same |
| Names with spaces and unicode | Copied | same |
| Thousands of named files | Not handled: the paths go on one command line (about 32 000 characters) | Not tested: a `shortcut:` in the code says when to change it |

### The setup script

| Scenario | Expected | Test |
|---|---|---|
| None | No command | `the_setup_script_is_found_and_powershell_comes_first` |
| `setup.cmd` only | `cmd.exe /c .nekominal\setup.cmd` | same |
| `setup.ps1`, or both | PowerShell runs `setup.ps1` | same |
| `.nekominal` is a file, or `setup.ps1` is a folder | Not taken for a script | `a_setup_script_must_be_a_file` |
| The default profile is Git Bash or WSL | The command line is written for cmd and PowerShell; bash reads `\s` as an escape | Not tested: a documented limit of `setup_command` |

### Workspace flows: `crates/nekominal-shell/src/app/workspace.rs`

| Scenario | Expected | Test |
|---|---|---|
| The palette line: empty, invalid, valid branch | A hint, the reason it is refused, or "Create" | The rule is `valid_branch` (above); the rows themselves need an `App`, which needs a window: not testable automatically |
| Remove from a pane with no folder | "not a repository" | Not testable automatically (an `App`) |
| The dialog was accepted but the worktree changed since | Inspected again; the dialog comes back when it can no longer go | Not testable automatically (an `App`); the model it re-runs is tested in remove_dialog.rs |
| Tabs in the folder close first, then a wait for their shells | As described | Not testable automatically (real panes); the git side is `a_worktree_a_process_still_sits_in_loses_its_branch_as_asked` |
| Show the tab | The focused pane first, else the first pane in the folder | Not testable automatically (an `App`) |

## The dialog: `crates/nekominal-shell/src/remove_dialog.rs`

| Scenario | Expected | Test |
|---|---|---|
| Every combination of: clean / changed / untracked only; no remote / remote; push state unknown / 0 / some; no default branch / merged / not merged; with or without a pull request, tabs, an agent | Both choices enabled exactly when nothing would be lost (no changes, and no unpushed commits to a remote); then the merged branch preselects Delete, else Archive; otherwise nothing is chosen and the main button shows the tab. One line per known fact, warnings only for changes and unpushed commits | `every_combination_of_checks_enables_the_choices_only_when_nothing_is_lost` |
| Clean and merged, with a PR | Delete preselected, "merged (PR #n)" | `a_clean_merged_worktree_offers_to_delete_the_branch_first` |
| Clean, not merged | Archive preselected | `a_clean_worktree_not_merged_keeps_its_branch_unless_asked` |
| Changes and unpushed commits | Both off with the reason, Show the tab | `changes_disable_both_choices_and_the_main_button_shows_the_tab` |
| Only unpushed commits | Both off, "Push first" | `commits_not_pushed_disable_both_choices_with_their_reason` |
| No remote | Nothing waits on a push | `a_repository_with_no_remote_has_nothing_to_push_to` |
| A remote, push state unknown | No line about pushing, not blocked | `every_combination_of_checks_enables_the_choices_only_when_nothing_is_lost` |
| Main checkout, detached HEAD | Refused before the dialog | `the_main_checkout_and_a_detached_head_are_refused_before_the_dialog` |
| 5, 6, 7 changed files; changes with no file names | Five named at most, the count says all of them; no list when none is named | `the_file_list_shows_five_files_and_the_count_says_all`, `changes_disable_both_choices_and_the_main_button_shows_the_tab` |
| Tab, arrows, a click | Move between the two choices only when both can be taken; a choice out of range or already taken changes nothing | `the_choice_moves_only_between_choices_that_can_be_taken`, `a_clean_worktree_not_merged_keeps_its_branch_unless_asked` |
| Enter / Space | The main button | Not testable automatically (`App::popup_key`) |
| Layout | The file list pushes the lines under it down; choices under the checks, buttons under the choices, Cancel left of the main button | `the_files_under_a_check_push_the_lines_below_them_down`, `the_dialog_renders_its_lines_choices_and_buttons` |
| Screen reader | Title, path, checks and files, a line each | `a_screen_reader_hears_the_title_path_checks_and_files` |
| English with a count of one ("1 commits are not pushed") | Reads in the plural | Not fixed: the strings have no plural forms anywhere; Traditional Chinese, the first language, has none to need |

## Pull requests: `crates/nekominal-shell/src/github.rs`

| Scenario | Expected | Test |
|---|---|---|
| Open, draft, merged, closed | Their state; a merged or closed PR is that even when it was a draft | `a_draft_failing_passing_and_merged_pull_request_read_as_such`, `missing_fields_and_nulls_read_as_nothing_or_as_defaults` |
| An unknown state | Read as open (or draft) | `an_unknown_state_reads_as_open` |
| Missing or null `isDraft`, `reviewDecision`, `mergeStateStatus`, `statusCheckRollup`; a rollup that is not a list | Defaults: not a draft, none, no checks | `missing_fields_and_nulls_read_as_nothing_or_as_defaults` |
| Missing number, number as text or negative, missing or empty URL, not an object, not JSON | No pull request | same, and `what_is_not_a_pull_request_is_none` |
| CheckRun: every status and conclusion; StatusContext: every state | Counted as gh counts them: SUCCESS, NEUTRAL, SKIPPED pass; FAILURE, ERROR, CANCELLED, TIMED_OUT, ACTION_REQUIRED, STARTUP_FAILURE fail; anything else (running, queued, STALE, no conclusion, unknown) is pending | `every_check_run_and_status_context_state_is_counted_as_gh_counts_it` |
| A rollup of thousands of checks | All counted | `a_huge_rollup_is_counted_in_full` |
| Asked at once for a new branch, not again for a minute, never twice at a time | As said | `gh_is_asked_at_once_then_not_again_for_a_minute_and_never_twice_at_a_time` |
| An answer for a branch no tab is on any more | Dropped | `an_answer_for_a_branch_no_tab_is_on_is_dropped_and_none_clears_what_was_known` |
| A PR that went away (gh says none) | Cleared, which counts as a change | same |
| gh missing, signed out, not a GitHub remote, no PR | No pull request | `gh_missing_signed_out_or_outside_github_gives_no_pull_request` |
| gh that never answers (a network that hangs) | Not handled: there is no timeout, so that repository and branch is not asked again until no tab is on it | Not testable without a network, and not fixed: gh's own HTTP timeouts end it in practice |
| The repository of a folder, of a worktree (`.git` file) | The nearest folder with a `.git` | `the_repository_of_a_folder_is_the_nearest_with_a_git`, `the_repository_of_a_worktree_is_its_own_folder` |
| A folder on a share or of another system | gh never runs: `agent::git_branch` gives no branch | `agent::tests::the_branch_is_read_from_the_head_of_the_repository_above_the_folder` |

## Ports: `crates/nekominal-shell/src/ports.rs`

| Scenario | Expected | Test |
|---|---|---|
| IPv4 table, rows cut short, empty buffer | The rows that fit | `the_rows_of_an_ipv4_table_give_their_pid_and_port` |
| IPv6 table, several rows, a count larger than the buffer | The rows that fit | `the_rows_of_an_ipv6_table_give_their_pid_and_port`, `an_ipv6_table_of_several_rows_and_a_short_buffer` |
| Byte order; garbage in the high bytes of the port DWORD | The low two bytes in network order | `the_port_is_the_low_two_bytes_in_network_order_whatever_the_high_ones` |
| The same port on IPv4 and IPv6, or twice | Once | `only_the_ports_of_the_job_count_each_once` |
| Processes outside the job | Not counted | same |
| Many ports | Ascending, once each | `many_ports_come_back_sorted_once_each` |
| A real listener on an ephemeral port (IPv4, IPv6) | Found; ephemeral ports are not filtered out | `a_port_this_process_listens_on_is_found`, `a_port_this_process_listens_on_over_ipv6_is_found` (skipped without IPv6) |

## Badges: `crates/nekominal-shell/src/app/badges.rs`

| Scenario | Expected | Test |
|---|---|---|
| Open × (no checks, passing, running, failing and every mix); draft, merged, closed whatever their checks | Failing wins over running, running over passing; draft, merged and closed by their state; only a failing badge has a count | `every_state_and_mix_of_checks_has_its_look`, `a_pull_request_looks_like_its_checks_and_the_ports_follow_it` |
| PR and ports | The PR first, then each port with its `http://localhost:` URL | `a_pull_request_looks_like_its_checks_and_the_ports_follow_it` |
| Width | Mark, text, and the bubble of the count | `badges_fill_a_line_then_wrap_and_add_a_line_each_to_the_row` |
| Wrapping: fits, exactly at the edge, wider than the room, no room at all | Next line when it does not fit; exactly at the edge stays; a badge wider than the room has a line of its own | `badges_fill_a_line_then_wrap_and_add_a_line_each_to_the_row`, `badges_wrap_at_the_exact_edge_and_with_no_room` |
| Row height with and without badges | A line each; nothing without | `badges_fill_a_line_then_wrap_and_add_a_line_each_to_the_row`, `layout::tests::a_row_with_badges_is_taller_and_the_rows_under_it_move_down_by_as_much` |
| Top strip and icons layouts | No room for badges | `layout::tests::the_strip_and_the_icons_take_no_room_for_badges`; that `badge_heights` is empty for them needs an `App`: not testable automatically |
| A click | The badge under the pointer, none in the gap or below | `a_click_on_a_badge_opens_its_pull_request_or_its_port` |
| A click after the list scrolls | The list does not scroll (a `shortcut:` in layout.rs); the places of the badges are taken again every frame from where they were drawn | Not testable automatically: needs a painted `App` |
| The title bar | The badges of the selected tab come first among its pieces, so they are the last to give way | Not testable automatically: `bar_pieces` needs an `App` with a focused pane |
| Set once, cleared | A change is logged once; none clears | `a_badge_is_set_once_and_cleared_with_none` |
| Text for the agents panel and the screen reader: none, ports only, PR only, both | `#4 merged · :3000 :5173`, parts left out when unknown | `the_text_of_the_badges_says_the_pull_request_then_the_ports`, `the_text_of_the_badges_with_a_pull_request_only_or_many_ports`, `access::tests::the_tabs_and_the_terminal_text_are_in_the_tree_and_the_terminal_has_the_keyboard`, `agents_panel::tests::an_agent_that_said_nothing_shows_only_what_is_known` |
