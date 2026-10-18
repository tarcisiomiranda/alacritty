# Upstream policy and related work

Checked on 2026-10-18 with the GitHub API and the current default branch.

The upstream [contribution rules](https://github.com/alacritty/alacritty/blob/master/CONTRIBUTING.md#llmai-contributions)
do not allow code or text made with LLM tools. The
[PR form](https://github.com/alacritty/alacritty/blob/master/.github/pull_request_template.md)
asks the author to state that no LLM was used. These local changes used AI help,
so that statement would not be true. No PR, issue or comment for this work was
sent to upstream.

The [upstream README](https://github.com/alacritty/alacritty#faq) also puts tabs
and split panes outside the goals of that project.

| Topic | Related work | State at the time of review |
| --- | --- | --- |
| Native tabs | [#3129](https://github.com/alacritty/alacritty/issues/3129) | Closed, wontfix; maintainers declined a tab PR |
| Split panes | [#1360](https://github.com/alacritty/alacritty/issues/1360) | Closed, wontfix |
| Menus | [#5604](https://github.com/alacritty/alacritty/issues/5604) | Closed |
| Close prompt | [#3426](https://github.com/alacritty/alacritty/issues/3426) | Closed, wontfix |
| tmux integration | [#2410](https://github.com/alacritty/alacritty/issues/2410) | Open; a different approach from native panes |
| Keyboard tab movement | [#8037](https://github.com/alacritty/alacritty/issues/8037) | Open; limited to macOS native tabs |
| Prompt tracking | [PR #8961](https://github.com/alacritty/alacritty/pull/8961) | Open; related OSC 133 work, not a native tab or session UI |

The search covered open issues and PRs about tabs, splits, saved sessions,
workspaces, context menus, settings, localization and close prompts. All 13 open
upstream PR titles were reviewed. No open PR for the same complete feature set
was found. Search results can miss work that uses different words, and project
state can change.

The user requested publication in their personal fork. The fork PRs keep that
work separate from upstream and do not make a false no-AI claim. Git commits
use the user's configured author identity and have no AI coauthor trailer.
