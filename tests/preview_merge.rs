use std::process::Command;

#[test]
fn merges_current_tips_and_rejects_conflicts() {
    let status = Command::new("bash")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .arg("-ceu")
        .arg(r#"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
git init -q -b main "$tmp/repo"
repo=(git -C "$tmp/repo" -c user.name=Test -c user.email=test@localhost)
echo base > "$tmp/repo/common"
"${repo[@]}" add .
"${repo[@]}" commit -qm base
"${repo[@]}" switch -qc feature
echo feature > "$tmp/repo/feature"
"${repo[@]}" add .
"${repo[@]}" commit -qm feature
"${repo[@]}" switch -q main
echo main > "$tmp/repo/main"
"${repo[@]}" add .
"${repo[@]}" commit -qm main
mkdir "$tmp/tree"
merge=(bash scripts/merge-preview.sh "$tmp/cache" "$tmp/repo" refs/heads/main refs/heads/feature "$tmp/tree")
first=$("${merge[@]}")
test -f "$tmp/tree/feature"
test -f "$tmp/tree/main"
test "$first" = "$("${merge[@]}")"
echo main-new > "$tmp/repo/common"
"${repo[@]}" commit -qam advance
test "$first" != "$("${merge[@]}")"
test "$(cat "$tmp/tree/common")" = main-new
"${repo[@]}" switch -q feature
echo conflicting > "$tmp/repo/common"
"${repo[@]}" commit -qam conflicting
if "${merge[@]}"; then exit 1; fi
test "$(cat "$tmp/tree/common")" = main-new
"#)
        .status()
        .expect("run preview merge regression");
    assert!(status.success());
}
