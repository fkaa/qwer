use std::{fs::{self, File}, io::BufReader, path::{Path, PathBuf}};

use anyhow::Context;
use askama::Template;
use comrak::{ComrakOptions, markdown_to_html};
use serde_json::Value;

#[derive(askama::Template)]
#[template(path = "page.html", escape = "none")]
struct HelpPageTemplate<'a> {
    menu_items: &'a [MenuItem],
    current_path: &'a str,
    content: &'a str,
}

struct MenuItem {
    name: String,
    path: String,
    children: Vec<MenuItem>,
}

enum TreeItem {
    Item { name: String, path: String, src_file: String, },
    Menu { name: String, path: String, items: Vec<TreeItem>, },
}

fn conv(tree: &TreeItem) -> MenuItem {
    match tree {
        TreeItem::Item { name, path, .. } => {
            MenuItem { name: name.clone(), path: path.clone(), children: Vec::new() }
        },
        TreeItem::Menu { name, path, items } => {
            let children = items.iter().map(conv).collect::<Vec<_>>();
            MenuItem { name: name.clone(), path: path.clone(), children }
        }
    }
}

pub fn generate_help_directory(tree: &Path, output: &Path) -> anyhow::Result<Vec<String>> {
    let mut src_paths = Vec::new();
    let tree_root = PathBuf::from(tree.parent().unwrap());

    src_paths.push(tree.as_os_str().to_str().unwrap().into());
    let tree = parse_tree_json(tree)?;
    let menu_items = tree.iter().map(conv).collect::<Vec<_>>();

    for item in tree {
        generate_help_item(&mut src_paths, &menu_items, tree_root.clone(), output.into(), &item)?;
    }

    Ok(src_paths)
}

fn generate_help_item(src_paths: &mut Vec<String>, menu_items: &[MenuItem], root_dir: PathBuf, dest_dir: PathBuf, item: &TreeItem) -> anyhow::Result<()> {
    let mut dest_path = dest_dir.clone();

    match item {
        TreeItem::Item { path, src_file, .. } => {
            dest_path.push(format!("{}", path));

            let mut src = root_dir.clone();
            src.push(src_file);

            let content = read_md_to_html(&src)?;
            src_paths.push(src.into_os_string().into_string().unwrap());

            let template = HelpPageTemplate {
                menu_items,
                current_path: &path,
                content: &content,
            };

            let content = template.render().unwrap();

            fs::create_dir_all(dest_path.parent().unwrap())?;
            fs::write(dest_path, content)?;
        },
        TreeItem::Menu { items, .. } => {
            // dest_path.push(format!("{}/", path));

            for item in items {
                generate_help_item(src_paths, menu_items, root_dir.clone(), dest_path.clone(), item)?;
            }
        }
    }

    Ok(())
}

fn read_md_to_html(src: &Path) -> anyhow::Result<String> {
    let md = fs::read_to_string(src).context(format!("reading from {:?}", src))?;

    Ok(markdown_to_html(&md, &ComrakOptions::default()))
}

fn parse_tree_json(path: &Path) -> anyhow::Result<Vec<TreeItem>> {
    let f = File::open(path)?;
    let mut reader = BufReader::new(f);

    let json: Value = serde_json::from_reader(&mut reader)?;

    let arr = json.as_array().unwrap();

    let mut items = Vec::new();
    for o in arr {
        items.push(parse_item(o)?);
    }

    Ok(items)
}

fn parse_item(value: &Value) -> anyhow::Result<TreeItem> {
    let title = value["title"].as_str().unwrap();
    let path = value["path"].as_str().unwrap();

    if let Some(obj) = value.get("src").and_then(|v| v.as_str()) {
        return Ok(TreeItem::Item { name: title.to_string(), path: path.to_string(), src_file: obj.to_string() });
    }

    if let Some(obj) = value.get("children").and_then(|v| v.as_array()) {
        let mut children = Vec::new();

        for child in obj {
            children.push(parse_item(child)?);
        }

        return Ok(TreeItem::Menu { name: title.to_string(), path: path.to_string(), items: children });
    }

    anyhow::bail!("unknown")
}
