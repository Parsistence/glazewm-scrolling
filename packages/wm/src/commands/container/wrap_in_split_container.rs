use std::collections::VecDeque;

use anyhow::Context;

use crate::{
  models::{Container, SplitContainer, TilingContainer, Workspace},
  traits::{CommonGetters, TilingSizeGetters},
};

/// Wraps the given children of `target_parent` in a split container.
///
/// The split container takes the combined size of the children. When
/// wrapping columns of a scrolling workspace, the split container instead
/// takes the width of the widest child, and the children share the split
/// container equally.
pub fn wrap_in_split_container(
  split_container: &SplitContainer,
  target_parent: &Container,
  target_children: &[TilingContainer],
) -> anyhow::Result<()> {
  let is_scrolling_parent = target_parent
    .as_workspace()
    .is_some_and(Workspace::is_scrolling);

  let starting_index = target_children
    .iter()
    .map(CommonGetters::index)
    .min()
    .context("Failed to get starting index.")?;

  target_parent
    .borrow_children_mut()
    .insert(starting_index, split_container.clone().into());

  let starting_focus_index = target_children
    .iter()
    .map(CommonGetters::focus_index)
    .min()
    .context("Failed to get starting focus index.")?;

  target_parent
    .borrow_child_focus_order_mut()
    .insert(starting_focus_index, split_container.id());

  // Get the total tiling size amongst all children.
  let total_tiling_size = target_children
    .iter()
    .map(TilingSizeGetters::tiling_size)
    .sum::<f32>();

  let target_children_ids = target_children
    .iter()
    .map(CommonGetters::id)
    .collect::<Vec<_>>();

  let sorted_focus_ids = target_parent
    .borrow_child_focus_order()
    .iter()
    .filter(|id| target_children_ids.contains(id))
    .copied()
    .collect::<VecDeque<_>>();

  // Set the split container's parent and tiling size.
  *split_container.borrow_parent_mut() = Some(target_parent.clone());
  split_container.set_tiling_size(if is_scrolling_parent {
    target_children
      .iter()
      .map(TilingSizeGetters::tiling_size)
      .fold(0.0, f32::max)
  } else {
    total_tiling_size
  });

  #[allow(clippy::cast_precision_loss)]
  let equal_share = 1.0 / target_children.len() as f32;

  // Move the children from their original parent to the split container.
  for target_child in target_children {
    *target_child.borrow_parent_mut() =
      Some(split_container.clone().into());

    split_container
      .borrow_children_mut()
      .push_back(target_child.clone().into());

    target_parent
      .borrow_children_mut()
      .retain(|child| child != &target_child.clone().into());

    target_parent
      .borrow_child_focus_order_mut()
      .retain(|id| id != &target_child.id());

    // Scale the tiling size to the new split container.
    target_child.set_tiling_size(if is_scrolling_parent {
      equal_share
    } else {
      target_child.tiling_size() / total_tiling_size
    });
  }

  // Add original focus order to split container.
  *split_container.borrow_child_focus_order_mut() = sorted_focus_ids;

  Ok(())
}
