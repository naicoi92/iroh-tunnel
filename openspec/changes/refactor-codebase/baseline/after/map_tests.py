"""Map removed baseline tests to post-refactor tests. Usage: map_tests.py <baseline dir>"""
import sys, collections
d = sys.argv[1]
# Manual mappings: leaf renamed/split/merged during the refactor (rationale in test-mapping.md).
MANUAL = {
 "lib::role_run::tests::protocol_str_matches_serde_form": ("lib::settings::tests::protocol_as_str_is_lowercase", "renamed: protocol_str moved to Protocol::as_str in engine settings"),
 "lib::service::scope::tests::elevated_defaults_to_system_without_a_prompt": ("lib::service::default_scope::tests::default_scope_follows_privilege", "merged: prompt-free defaults covered by default_scope (decision logic moved out of the prompt)"),
 "lib::service::scope::tests::non_interactive_unprivileged_gets_the_user_scope": ("lib::service::default_scope::tests::default_scope_follows_privilege", "merged: same default_scope test"),
 "lib::service::scope::tests::explicit_scope_is_never_overridden_or_prompted": ("lib::service::decide_install_scope::tests::explicit_scope_is_never_overridden_or_asked", "renamed"),
 "lib::service::scope::tests::interactive_user_is_offered_both_scopes": ("lib::service::decide_install_scope::tests::install_scope_needs_choice_when_caller_can_answer", "renamed: now returns a structured scope-choice need instead of prompting"),
 "lib::service::scope::tests::unknown_or_missing_answer_is_an_error_not_a_guess": ("lib::service::parse_scope_answer::tests::unknown_answer_is_a_config_error_not_a_guess", "renamed; answer parsing split out"),
 "lib::service::system::tests::another_users_service_needs_an_explicit_yes": ("lib::service::system::tests::retarget_without_consent_needs_confirmation", "renamed: prompt replaced by structured needs-confirmation"),
 "lib::service::system::tests::interactive_unknown_user_is_asked_and_a_bad_answer_is_an_error": ("lib::service::system::tests::target_account_needs_invoking_user_instead_of_prompting", "replaced: operations return a structured need rather than prompting; bad-answer parsing covered by parse_scope_answer test"),
 "lib::service::system::tests::nothing_installed_or_same_user_needs_no_confirmation": ("lib::service::system::tests::stale_retarget_consent_is_not_applied", "replaced: confirmation policy tests restructured around consent; no-confirmation path exercised by service_user integration tests"),
 "lib::service::system::tests::retarget_without_a_terminal_fails_instead_of_assuming_yes": ("lib::service::system::tests::retarget_without_consent_needs_confirmation", "merged: no terminal -> needs-confirmation outcome"),
 "lib::service::system::tests::unknown_user_without_a_terminal_or_config_is_an_actionable_error": ("lib::service::system::tests::unknown_user_error_names_the_rerun_and_config_options", "renamed"),
 "lib::service::system::tests::unpinned_registration_counts_as_another_owner": ("lib::service::system::tests::retarget_from_unpinned_registration_matches_existing_policy", "renamed"),
 "lib::service::tests::resolve_binary_returns_a_path": ("lib::cli_service::tests::cli_resolves_tunnel_binary_from_path_first", "moved: binary resolution now lives in cli_service (host); the old test only asserted a path was returned"),
 "lib::shutdown::tests::begin_publishes_the_same_deadline_to_forced_exit": ("lib::runtime::run_context::tests::begin_publishes_to_its_own_context", "replaced: process-global drain deadline became per-run context (also first_published_deadline_wins, two_runs_do_not_share_a_deadline)"),
}
def rd(p): return [l.rstrip("\n") for l in open(p) if l.strip()]
def run(name):
    old, new = set(rd(f"{d}/{name}")), set(rd(f"{d}/after/{name}"))
    rem, add = sorted(old - new), sorted(new - old)
    leaf = collections.defaultdict(list)
    for n in add: leaf[n.rsplit("::", 1)[1]].append(n)
    rows, un = [], []
    for r in rem:
        tgt, rest = r.split("::", 1)
        l = rest.rsplit("::", 1)[1]
        c = [a for a in leaf.get(l, []) if a.split("::", 1)[0] == tgt]
        if len(c) > 1:  # disambiguate by shared module-path tokens
            toks = set(rest.split("::")[:-1])
            c.sort(key=lambda a: -len(toks & set(a.split("::")[1:-1])))
            c = c[:1]
        if not c and r in MANUAL:
            assert MANUAL[r][0] in add, r
            c = [MANUAL[r][0]]
        (rows if c else un).append((r, c[0] if c else None))
    return len(old), len(new), rem, add, rows, un
for name in ("tests.txt", "tests-ignored.txt"):
    o, n, rem, add, rows, un = run(name)
    mapped_new = {b for _, b in rows}
    print(f"## {name}: before={o} after={n} removed={len(rem)} added={len(add)} mapped={len(rows)} unmapped={len(un)}")
    print("pure-additions (new names not a mapping target):", len(set(add) - mapped_new))
    for u, _ in un: print("UNMAPPED", u)
    if "-v" in sys.argv:
        for a, b in rows: print(f"| `{a}` | `{b}` | " + (MANUAL[a][1] if a in MANUAL else "same leaf name, module path changed" if a.split("::",1)[0]!="lib" or True else ""))
        print("ADDED-NEW")
        for a in sorted(set(add) - mapped_new): print(f"- `{a}`")
