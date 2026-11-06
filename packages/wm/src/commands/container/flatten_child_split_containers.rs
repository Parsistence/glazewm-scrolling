use super::flatten_split_container;
use crate::{
  models::{Container, DirectionContainer, SplitContainer, Workspace},
  traits::{CommonGetters, TilingDirectionGetters},
};

/// Flattens any redundant split containers at the top-level of the given
/// parent container.
///
/// For example:
/// ```ignore,compile_fail
/// H[1 H[V[2, 3]]] -> H[1, 2, 3]
/// H[1 H[2, 3]] -> H[1, 2, 3]
/// H[V[1]] -> V[1]
/// ```
///
/// For scrolling workspaces, columns that are split containers with a
/// single child are flattened instead of inverting the workspace's tiling
/// direction, since it must stay horizontal.
pub fn flatten_child_split_containers(
  parent: &Container,
) -> anyhow::Result<()> {
  let Ok(parent) = parent.as_direction_container() else {
    return Ok(());
  };

  let is_scrolling_workspace =
    parent.as_workspace().is_some_and(Workspace::is_scrolling);

  if is_scrolling_workspace {
    let single_child_columns = split_children(&parent)
      .into_iter()
      .filter(|split_child| split_child.child_count() == 1);

    for column in single_child_columns {
      flatten_split_container(column)?;
    }

    return flatten_same_direction_children(&parent);
  }

  // Get children that are either tiling windows or split containers.
  let tiling_children = parent
    .children()
    .into_iter()
    .filter(|child| child.is_tiling_window() || child.is_split())
    .collect::<Vec<_>>();

  if tiling_children.len() == 1 {
    // Handle case where the parent is a split container and has a
    // single split container child.
    if let Some(split_child) = tiling_children[0].as_split() {
      flatten_split_container(split_child.clone())?;
      parent.set_tiling_direction(parent.tiling_direction().inverse());
    }

    return Ok(());
  }

  flatten_same_direction_children(&parent)
}

/// Gets the split container children of the given parent.
fn split_children(parent: &DirectionContainer) -> Vec<SplitContainer> {
  parent
    .children()
    .into_iter()
    .filter_map(|child| child.as_split().cloned())
    .collect()
}

/// Flattens split container children that have the same tiling direction
/// as their parent.
fn flatten_same_direction_children(
  parent: &DirectionContainer,
) -> anyhow::Result<()> {
  for split_child in split_children(parent).iter().filter(|split_child| {
    split_child.tiling_direction() == parent.tiling_direction()
  }) {
    // Additionally flatten redundant top-level split containers in the
    // child.
    if split_child.child_count() == 1 {
      if let Some(split_grandchild) = split_child.children()[0].as_split()
      {
        flatten_split_container(split_grandchild.clone())?;
      }
    }

    flatten_split_container(split_child.clone())?;
  }

  Ok(())
}
