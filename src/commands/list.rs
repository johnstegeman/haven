/// List all tracked files with their decoded destination paths and flags.
use anyhow::Result;
use std::path::Path;

use crate::ignore::IgnoreList;
use crate::source;

pub struct ListOptions<'a> {
    pub repo_root: &'a Path,
}

pub fn run(opts: &ListOptions<'_>) -> Result<()> {
    let source_dir = opts.repo_root.join("source");
    let ignore = IgnoreList::load(opts.repo_root);
    let entries = source::scan(&source_dir, &ignore)?;

    if entries.is_empty() {
        println!("No files tracked. Run `dfiles add <file>` to start tracking.");
        return Ok(());
    }

    for entry in &entries {
        // Collect flag tags in a consistent display order.
        let mut tags: Vec<&str> = Vec::new();
        if entry.flags.template  { tags.push("template"); }
        if entry.flags.symlink   { tags.push("symlink"); }
        if entry.flags.private   { tags.push("private"); }
        if entry.flags.executable{ tags.push("executable"); }
        if entry.flags.extdir    { tags.push("extdir"); }
        if entry.flags.extfile   { tags.push("extfile"); }
        if entry.flags.create_only { tags.push("create-only"); }
        if entry.flags.exact     { tags.push("exact"); }

        if tags.is_empty() {
            println!("{}", entry.dest_tilde);
        } else {
            println!("{}  ({})", entry.dest_tilde, tags.join(", "));
        }
    }

    Ok(())
}
