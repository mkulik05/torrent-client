
use egui::{CollapsingHeader, Ui};

#[derive(Clone, Default)]
struct Tree {
    children: Vec<(Tree, String)>,
    end_nodes: Vec<String>,
}

impl Tree {
    fn build_tree(paths: Vec<&str>) -> Self {
        let mut root = Tree::default();

        for path in paths {
            let parts: Vec<&str> = path.split('/').collect();
            let mut current_node = &mut root;

            for part in parts.iter().take(parts.len() - 1) {
                let child_index = current_node.children.iter().position(|(_, name)| *name == *part);
                match child_index {
                    Some(index) => current_node = &mut current_node.children[index].0,
                    None => {
                        let new_node = Tree::default();
                        current_node.children.push((new_node, part.to_string()));
                        current_node = &mut current_node.children.last_mut().unwrap().0;
                    }
                }
            }

            if let Some(file_name) = parts.last() {
                current_node.end_nodes.push((*file_name).to_string());
            }
        }

        root
    }
}

impl Tree {
    fn draw(&mut self, ui: &mut Ui) {
        self.ui_impl(ui, 0);
    }
    fn ui_impl(&mut self, ui: &mut Ui, depth: usize) {
        for (child,  name) in &mut self.children {
            CollapsingHeader::new((*name).to_string())
                .default_open(depth < 1)
                .show(ui, |ui| child.ui_impl(ui, depth + 1));
        }
        for name in &self.end_nodes {
            ui.label(name);
        }

    }
}

pub fn draw_tree(pathes: &Vec<&str>, root_name: String, ui: &mut Ui) {
    let tree: Tree = Tree::build_tree(pathes.to_vec());
    let mut tree = Tree {
        children: vec![(tree, root_name)],
        end_nodes: Vec::new()
    };
    tree.draw(ui);
}