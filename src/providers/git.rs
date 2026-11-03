//! Provider implementations; shared policy remains in the parent module.
use super::*;

pub(super) fn collect_git(query: &ProjectQuery, cancelled_flag: &AtomicBool) -> ProviderResult {
    let ctx = parse_git_context(query);
    let needs = git_needs(&ctx, provider_scope(query, "git"));
    let mut result = ProviderResult {
        project_root: git_project_root(&ctx),
        ..ProviderResult::default()
    };
    git_watch_paths(&ctx, &mut result, needs.status_paths);
    if (ctx.subcommand.is_empty() && query.provider.is_none())
        || (!needs.refs && !needs.remotes && !needs.worktrees && !needs.status_paths)
    {
        return result;
    }
    let mut deadline = Instant::now() + GIT_DEADLINE;
    let mut refs = GitRefs::default();
    if needs.refs {
        let output = run_fixed_git(
            query,
            &ctx,
            &[
                "for-each-ref",
                "--format=%(refname)",
                "--sort=refname",
                "refs/heads",
                "refs/remotes",
                "refs/tags",
            ],
            &mut deadline,
            cancelled_flag,
        );
        append_git_process_diagnostic(&mut result, "引用", &output);
        if !output.interrupted() && !output.failed() {
            refs = parse_git_refs(&output.stdout);
        }
    }
    if needs.remotes && !cancelled(cancelled_flag) && Instant::now() < deadline {
        let output = run_fixed_git(query, &ctx, &["remote"], &mut deadline, cancelled_flag);
        append_git_process_diagnostic(&mut result, "远端", &output);
        if !output.interrupted() && !output.failed() {
            refs.remotes.extend(
                output
                    .stdout
                    .split(|byte| *byte == b'\n' || *byte == b'\r')
                    .filter_map(|line| String::from_utf8(line.to_vec()).ok())
                    .map(|line| line.trim().to_owned())
                    .filter(|line| !line.is_empty()),
            );
        }
    }
    if needs.worktrees && !cancelled(cancelled_flag) && Instant::now() < deadline {
        let output = run_fixed_git(
            query,
            &ctx,
            &["worktree", "list", "--porcelain"],
            &mut deadline,
            cancelled_flag,
        );
        append_git_process_diagnostic(&mut result, "工作树", &output);
        if !output.interrupted() && !output.failed() {
            for path in parse_worktrees(&output.stdout) {
                add_filtered(
                    &mut result,
                    path,
                    "Git 工作树目录",
                    CandidateKind::Directory,
                    "git.worktree",
                    &query.prefix,
                );
            }
        }
    }
    if needs.status_paths && !cancelled(cancelled_flag) && Instant::now() < deadline {
        let output = run_fixed_git(
            query,
            &ctx,
            &["status", "--short", "--untracked-files=all", "-z"],
            &mut deadline,
            cancelled_flag,
        );
        append_git_process_diagnostic(&mut result, "状态路径", &output);
        if !output.interrupted() && !output.failed() {
            for path in parse_status_paths(&output.stdout) {
                add_filtered(
                    &mut result,
                    path,
                    "Git 状态中的路径",
                    CandidateKind::File,
                    "git.status_path",
                    &query.prefix,
                );
            }
        }
    }
    if needs.refs {
        if needs.branches {
            for branch in &refs.local_branches {
                add_filtered(
                    &mut result,
                    branch.clone(),
                    "本地 Git 分支",
                    CandidateKind::Value,
                    "git.branch",
                    &query.prefix,
                );
            }
        }
        if needs.remote_branches {
            for branch in &refs.remote_branches {
                add_filtered(
                    &mut result,
                    branch.clone(),
                    "远端 Git 分支",
                    CandidateKind::Value,
                    "git.remote_branch",
                    &query.prefix,
                );
            }
        }
        if needs.tags {
            for tag in &refs.tags {
                add_filtered(
                    &mut result,
                    tag.clone(),
                    "Git 标签",
                    CandidateKind::Value,
                    "git.tag",
                    &query.prefix,
                );
            }
        }
    }
    if needs.remotes {
        for remote in refs.remotes {
            add_filtered(
                &mut result,
                remote,
                "Git 远端名称",
                CandidateKind::Value,
                "git.remote",
                &query.prefix,
            );
        }
    }
    if cancelled(cancelled_flag) {
        result.incomplete = true;
        result.diagnostics.push("Git 项目查询已取消".to_owned());
    }
    result.finish()
}
