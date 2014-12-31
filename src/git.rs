extern crate git2;

use prompt_buffer;
use prompt_buffer::{PromptLine, PromptBufferPlugin, PromptLineBuilder};
use git2::{Repository, Error, StatusOptions, STATUS_WT_NEW};
use std::{os, fmt};
use term::color;

enum StatusTypes {
    New,
    Modified,
    Deleted,
    Renamed,
    TypeChange,
    Untracked,
    Clean
}

impl fmt::Show for StatusTypes {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", match *self {
            StatusTypes::New => "A",
            StatusTypes::Modified => "M",
            StatusTypes::Deleted => "D",
            StatusTypes::Renamed => "R",
            StatusTypes::TypeChange => "T",
            StatusTypes::Untracked => "?",
            StatusTypes::Clean => " "
        })
    }
}

struct GitStatus {
    index: StatusTypes,
    workdir: StatusTypes
}

impl GitStatus {
    fn new(f: git2::Status) -> GitStatus {
        GitStatus {
            index:
                     if f.contains(git2::STATUS_INDEX_NEW) { StatusTypes::New }
                else if f.contains(git2::STATUS_INDEX_MODIFIED) { StatusTypes::Modified }
                else if f.contains(git2::STATUS_INDEX_DELETED) { StatusTypes::Deleted }
                else if f.contains(git2::STATUS_INDEX_RENAMED) { StatusTypes::Renamed }
                else if f.contains(git2::STATUS_INDEX_TYPECHANGE) { StatusTypes::TypeChange }
                else if f.contains(git2::STATUS_WT_NEW) { StatusTypes::Untracked }
                else { StatusTypes::Clean },
            workdir:
                     if f.contains(git2::STATUS_WT_NEW) { StatusTypes::Untracked }
                else if f.contains(git2::STATUS_WT_MODIFIED) { StatusTypes::Modified }
                else if f.contains(git2::STATUS_WT_DELETED) { StatusTypes::Deleted }
                else if f.contains(git2::STATUS_WT_RENAMED) { StatusTypes::Renamed }
                else if f.contains(git2::STATUS_WT_TYPECHANGE) { StatusTypes::TypeChange }
                else { StatusTypes::Clean },
        }
    }
}

impl fmt::Show for GitStatus {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}{}", self.index, self.workdir)
    }
}

fn get_git(path: &Path) -> Option<Repository> {
    match Repository::discover(path) {
        Ok(repo) => Some(repo),
        _ => None
    }
}

fn status(buffer: &mut Vec<PromptLine>, path: &Path, repo: &Repository) -> bool {
    let st = repo.statuses(Some(StatusOptions::new()
        .include_untracked(true)
        .renames_head_to_index(true)
        .exclude_submodules(true)
    ));

    let make_path_relative = |current: Path| {
        let mut fullpath = repo.workdir().unwrap();
        fullpath.push(current);

        fullpath.path_relative_from(path).unwrap()
    };

    match st {
        Ok(statuses) => {
            if statuses.len() <= 0 { return false }

            buffer.push(PromptLineBuilder::new()
                .colored_block(&"Git Status", color::CYAN)
                .build());

            for stat in statuses.iter() {
                let mut line = PromptLineBuilder::new_free();

                let status = GitStatus::new(stat.status());

                let diff = match stat.head_to_index() {
                    Some(delta) => Some(delta),
                    None => match stat.index_to_workdir() {
                        Some(delta) => Some(delta),
                        None => None
                    }
                };

                let val = format!("{} {}", status, match diff {
                    Some(delta) => {
                        let old = make_path_relative(delta.old_file().path().unwrap());
                        let new = make_path_relative(delta.new_file().path().unwrap());

                        if old != new {
                            format!("{} -> {}", old.display(), new.display())
                        } else {
                            format!("{}", old.display())
                        }
                    },
                    None => format!("{}", Path::new(stat.path().unwrap()).display())
                });

                line = match status.index {
                    StatusTypes::Clean => line.colored_block(&val, file_state_color(status.workdir)),
                    _ => match status.workdir {
                        StatusTypes::Clean | StatusTypes::Untracked =>
                            line.bold_colored_block(&val, file_state_color(status.index)),
                        _ => line.bold_colored_block(&val, color::RED)
                    }
                };

                buffer.push(line.indent().build());
            }

            return true
        },
        _ => { return false }
    }

    fn file_state_color(state: StatusTypes) -> u16 {
        match state {
            StatusTypes::Clean | StatusTypes::Untracked => color::WHITE,
            StatusTypes::Deleted => color::RED,
            StatusTypes::Modified => color::BLUE,
            StatusTypes::New => color::GREEN,
            StatusTypes::Renamed => color::CYAN,
            StatusTypes::TypeChange => color::YELLOW,
        }
    }
}

struct BranchInfo {
    name: Option<String>,
    upstream: Option<String>
}

fn git_branch(repo: &Repository) -> Result<BranchInfo, git2::Error> {
    let mut branches = repo.branches(None).ok().expect("Unable to load branches");

    for (mut branch, _) in branches {
        if !branch.is_head() {
            continue;
        }

        let name = branch.name();
        return Ok(BranchInfo {
            name: match name {
                Ok(n) => match n {
                    Some(value) => Some(value.to_string()),
                    _ => None
                },
                _ => None
            },
            upstream: match branch.upstream() {
                Ok(upstream) => {
                    match upstream.name() {
                        Ok(n) => match n {
                            Some(value) => Some(value.to_string()),
                            _ => None
                        },
                        _ => None
                    }
                },
                Err(_) => None
            }
        });
    }

    match repo.head() {
        Ok(r) => match repo.find_object(r.target().unwrap(), None) {
            Ok(obj) => {
                let sid = obj.short_id().ok().unwrap();
                let s = sid.as_str();
                let short_id = s.unwrap();
                Ok(BranchInfo {
                    name: Some(format!("{}", short_id)),
                    upstream: Some("?".to_string())
                })
            },
            Err(e) => Err(e)
        },
        Err(e) => Err(e)
    }
}

fn outgoing(buffer: &mut Vec<PromptLine>, repo: &Repository, has_status: bool) -> bool {
    match do_outgoing(buffer, repo, has_status) {
        Ok(r) => r,
        Err(e) => {
            println!("Error from outgoing: {}", e);
            false
        }
    }
}

fn do_outgoing(buffer: &mut Vec<PromptLine>, repo: &Repository, has_status: bool) -> Result<bool, git2::Error> {
    let branches = try!(git_branch(repo));

    let mut revwalk = try!(repo.revwalk());
    revwalk.set_sorting(git2::SORT_REVERSE);

    let from = try!(repo.revparse_single(branches.upstream.unwrap().as_slice())).id();
    let to = try!(repo.revparse_single(branches.name.unwrap().as_slice())).id();

    try!(revwalk.push(to));
    try!(revwalk.hide(from));

    let mut log_shown = false;

    for id in revwalk {
        let mut commit = try!(repo.find_commit(id));

        if !log_shown {
            buffer.push(PromptLineBuilder::new()
                .colored_block(&"Git Outgoing", color::CYAN)
                .indent_by(if has_status { 1 } else { 0 })
                .build());
            log_shown = true;
        }

        buffer.push(PromptLineBuilder::new_free()
            .indent()
            .colored_block(&format!("{} {}",
                String::from_utf8_lossy(
                    try!(try!(repo.find_object(commit.id(), None)).short_id()).get()
                ),
                String::from_utf8_lossy(match commit.summary_bytes() {
                    Some(b) => b,
                    None => continue
                })), color::WHITE)
            .build());
    }

    return Ok(log_shown);
}

fn end(buffer: &mut Vec<PromptLine>, repo: &Repository, indented: bool) {
    match git_branch(repo) {
        Ok(branches) => {
            buffer.push(PromptLineBuilder::new()
                .colored_block(
                    &match (branches.name, branches.upstream) {
                        (None, None) => "New Repository".to_string(),
                        (Some(name), None) => name,
                        (Some(name), Some(remote)) => format!("{}{} -> {}{}",
                            name,
                            prompt_buffer::reset(),
                            prompt_buffer::col(color::MAGENTA),
                            remote),
                        _ => "Unknown branch state".to_string()
                    }, color::CYAN)
                .indent_by(if indented { 1 } else { 0 })
                .build());
        },
        Err(_) => {}
    };
}

pub struct GitPlugin {
    repo: Option<Repository>,
    path: Path
}

impl GitPlugin {
    pub fn new() -> GitPlugin {
        GitPlugin {
            repo: None,
            path: os::make_absolute(&Path::new(".")).unwrap()
        }
    }
}

impl PromptBufferPlugin for GitPlugin {
    fn run(&mut self, path: &Path, lines: &mut Vec<PromptLine>) {
        if self.path != *path || self.repo.is_none() {
            self.path = path.clone();
            self.repo = get_git(&self.path);
        }

        match self.repo {
            Some(ref r) => {
                let st = status(lines, path, r);
                let out = outgoing(lines, r, st);
                end(lines, r, st || out);
            },
            _ => { }
        }
    }
}
