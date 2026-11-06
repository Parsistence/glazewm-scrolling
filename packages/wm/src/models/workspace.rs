use std::{
  cell::{Ref, RefCell, RefMut},
  collections::VecDeque,
  rc::Rc,
};

use anyhow::Context;
use uuid::Uuid;
use wm_common::{
  ColumnDto, ContainerDto, GapsConfig, ScrollingConfig, ScrollingStateDto,
  TilingDirection, WorkspaceConfig, WorkspaceDto, WorkspaceLayout,
};
use wm_platform::{LengthUnit, LengthValue, Rect, RectDelta};

use crate::{
  impl_common_getters, impl_container_debug,
  impl_tiling_direction_getters,
  models::{
    Container, DirectionContainer, TilingContainer, WindowContainer,
  },
  scrolling::{self, ColumnSpan, ColumnVisibility},
  traits::{
    CommonGetters, PositionGetters, TilingDirectionGetters,
    TilingSizeGetters,
  },
};

#[derive(Clone)]
pub struct Workspace(Rc<RefCell<WorkspaceInner>>);

#[derive(Debug)]
struct WorkspaceInner {
  id: Uuid,
  parent: Option<Container>,
  children: VecDeque<Container>,
  child_focus_order: VecDeque<Uuid>,
  config: WorkspaceConfig,
  gaps_config: GapsConfig,
  tiling_direction: TilingDirection,
  layout: WorkspaceLayout,
  scrolling_config: ScrollingConfig,

  /// Horizontal scroll position of the viewport within the column strip
  /// (in pixels). Only used by the scrolling layout.
  scroll_offset: i32,

  /// Layout and scroll state that was last broadcast over IPC. Used to
  /// only emit events when the state changes.
  broadcast_scroll_state:
    Option<(WorkspaceLayout, Option<ScrollingStateDto>)>,
}

impl Workspace {
  /// Creates a workspace using the layout from its config.
  ///
  /// Workspaces with the scrolling layout always tile horizontally,
  /// regardless of the given tiling direction.
  pub fn new(
    config: WorkspaceConfig,
    gaps_config: GapsConfig,
    scrolling_config: ScrollingConfig,
    tiling_direction: TilingDirection,
  ) -> Self {
    let layout = config.layout;
    let tiling_direction = match layout {
      WorkspaceLayout::Scrolling => TilingDirection::Horizontal,
      WorkspaceLayout::Tiling => tiling_direction,
    };

    let workspace = WorkspaceInner {
      id: Uuid::new_v4(),
      parent: None,
      children: VecDeque::new(),
      child_focus_order: VecDeque::new(),
      config,
      gaps_config,
      tiling_direction,
      layout,
      scrolling_config,
      scroll_offset: 0,
      broadcast_scroll_state: None,
    };

    Self(Rc::new(RefCell::new(workspace)))
  }

  /// Underlying config for the workspace.
  pub fn config(&self) -> WorkspaceConfig {
    self.0.borrow().config.clone()
  }

  /// Update the underlying config for the workspace.
  pub fn set_config(&self, config: WorkspaceConfig) {
    self.0.borrow_mut().config = config;
  }

  /// Whether the workspace is currently displayed by the parent monitor.
  pub fn is_displayed(&self) -> bool {
    self
      .monitor()
      .and_then(|monitor| monitor.displayed_workspace())
      .is_some_and(|workspace| workspace.id() == self.id())
  }

  pub fn set_gaps_config(&self, gaps_config: GapsConfig) {
    self.0.borrow_mut().gaps_config = gaps_config;
  }

  /// Current layout of the workspace.
  pub fn layout(&self) -> WorkspaceLayout {
    self.0.borrow().layout
  }

  /// Sets the layout of the workspace.
  ///
  /// This does not adjust the sizes of child containers; use the
  /// `set_layout` command for that.
  pub fn set_layout(&self, layout: WorkspaceLayout) {
    self.0.borrow_mut().layout = layout;
  }

  /// Whether the workspace uses the scrolling layout.
  pub fn is_scrolling(&self) -> bool {
    self.layout() == WorkspaceLayout::Scrolling
  }

  pub fn set_scrolling_config(&self, scrolling_config: ScrollingConfig) {
    self.0.borrow_mut().scrolling_config = scrolling_config;
  }

  /// Horizontal scroll position of the viewport (in pixels).
  pub fn scroll_offset(&self) -> i32 {
    self.0.borrow().scroll_offset
  }

  pub fn set_scroll_offset(&self, scroll_offset: i32) {
    self.0.borrow_mut().scroll_offset = scroll_offset;
  }

  /// Size (fraction of the viewport) for newly created columns.
  pub fn default_column_size(&self) -> f32 {
    let default_width = self
      .0
      .borrow()
      .scrolling_config
      .default_column_width
      .clone();

    self.column_size_from_length(&default_width)
  }

  /// Column sizes (fractions of the viewport) of the configured width
  /// presets.
  pub fn column_size_presets(&self) -> Vec<f32> {
    let presets = self
      .0
      .borrow()
      .scrolling_config
      .column_width_presets
      .clone();

    presets
      .iter()
      .map(|preset| self.column_size_from_length(preset))
      .collect()
  }

  /// Converts a length to a column size. Percentages are relative to the
  /// viewport width.
  ///
  /// Pixel lengths fall back to half the viewport if the workspace is not
  /// yet attached to a monitor.
  pub fn column_size_from_length(&self, length: &LengthValue) -> f32 {
    match length.unit {
      LengthUnit::Percentage => {
        scrolling::clamp_column_size(length.amount)
      }
      LengthUnit::Pixel => self.to_rect().map_or(0.5, |rect| {
        scrolling::column_size_from_px(
          length.to_px(rect.width(), None),
          rect.width(),
          self.column_gap(),
        )
      }),
    }
  }

  /// Gap between adjacent columns in pixels.
  ///
  /// Mirrors the horizontal gap from `TilingSizeGetters::inner_gaps`.
  pub fn column_gap(&self) -> i32 {
    let Some(monitor) = self.monitor() else {
      return 0;
    };

    let monitor_rect = monitor.native_properties().bounds;
    let gaps_config = &self.0.borrow().gaps_config;

    let scale_factor = if gaps_config.scale_with_dpi {
      monitor.native_properties().scale_factor
    } else {
      1.
    };

    gaps_config
      .inner_gap
      .to_px(monitor_rect.height(), Some(scale_factor))
  }

  /// Gets the spans of the workspace's columns within the strip, in
  /// left-to-right order.
  pub fn column_spans(
    &self,
  ) -> anyhow::Result<Vec<(TilingContainer, ColumnSpan)>> {
    let viewport = self.to_rect()?;
    let columns = self.tiling_children().collect::<Vec<_>>();

    let sizes = columns
      .iter()
      .map(TilingSizeGetters::tiling_size)
      .collect::<Vec<_>>();

    let spans =
      scrolling::column_spans(&sizes, viewport.width(), self.column_gap());

    Ok(columns.into_iter().zip(spans).collect())
  }

  /// Gets the column (direct tiling child) that contains the given
  /// container, if the container is in this workspace.
  pub fn column_of(
    &self,
    container: &Container,
  ) -> Option<TilingContainer> {
    container
      .self_and_ancestors()
      .find(|ancestor| {
        ancestor
          .parent()
          .is_some_and(|parent| parent.id() == self.id())
      })
      .and_then(|column| column.as_tiling_container().ok())
  }

  /// Gets where to insert a tiling container that should be placed next
  /// to `reference` (typically the last focused tiling window).
  ///
  /// In scrolling workspaces, the container becomes a new column after
  /// the reference's column. Otherwise, it becomes the reference's next
  /// sibling. Without a reference within this workspace, the container is
  /// appended to the workspace.
  ///
  /// Returns a tuple of (parent container, insertion index).
  pub fn tiling_insertion_point(
    &self,
    reference: Option<&Container>,
  ) -> (Container, usize) {
    let reference = reference
      .filter(|reference| {
        reference
          .workspace()
          .is_some_and(|workspace| workspace.id() == self.id())
      })
      .map(|reference| {
        if self.is_scrolling() {
          self.column_of(reference).map(Into::into)
        } else {
          Some(reference.clone())
        }
      });

    reference
      .flatten()
      .and_then(|reference| {
        Some((reference.parent()?, reference.index() + 1))
      })
      .unwrap_or_else(|| (self.clone().into(), self.child_count()))
  }

  /// Gets the most recently focused column.
  pub fn focused_column(&self) -> Option<TilingContainer> {
    self
      .child_focus_order()
      .find_map(|child| child.as_tiling_container().ok())
  }

  /// Gets how much of the given column is within the viewport.
  ///
  /// Returns `None` if the container is not a column of this workspace.
  pub fn column_visibility(
    &self,
    column: &TilingContainer,
  ) -> anyhow::Result<Option<ColumnVisibility>> {
    let viewport_width = self.to_rect()?.width();
    let offset = self.scroll_offset();

    Ok(
      self
        .column_spans()?
        .into_iter()
        .find(|(candidate, _)| candidate.id() == column.id())
        .map(|(_, span)| {
          scrolling::column_visibility(span, offset, viewport_width)
        }),
    )
  }

  /// Moves a rect of a column that lies outside the viewport into the
  /// viewport. See [`scrolling::parked_x`].
  pub fn parked_column_rect(
    &self,
    column_rect: &Rect,
  ) -> anyhow::Result<Rect> {
    let viewport = self.to_rect()?;

    Ok(Rect::from_xy(
      scrolling::parked_x(
        column_rect.x(),
        column_rect.width(),
        viewport.x(),
        viewport.width(),
      ),
      column_rect.y(),
      column_rect.width(),
      column_rect.height(),
    ))
  }

  /// Gets a rect for the given container that lies fully on-screen.
  ///
  /// Rects of containers in columns that aren't fully within the viewport
  /// are parked within the viewport. Other rects are returned as-is.
  pub fn on_screen_rect(
    &self,
    container: &Container,
    rect: Rect,
  ) -> anyhow::Result<Rect> {
    let Some(column) = self.column_of(container) else {
      return Ok(rect);
    };

    match self.column_visibility(&column)? {
      Some(ColumnVisibility::Full) | None => Ok(rect),
      Some(_) => self.parked_column_rect(&rect),
    }
  }

  /// Scrolls the viewport just enough to fully reveal the focused column,
  /// and keeps the viewport within the bounds of the strip.
  ///
  /// Returns whether the scroll offset changed.
  pub fn update_scroll_offset(&self) -> anyhow::Result<bool> {
    let viewport_width = self.to_rect()?.width();
    let column_spans = self.column_spans()?;
    let current_offset = self.scroll_offset();

    let focused_span = self.focused_column().and_then(|focused| {
      column_spans
        .iter()
        .find(|(column, _)| column.id() == focused.id())
        .map(|(_, span)| *span)
    });

    let revealed_offset = focused_span.map_or(current_offset, |span| {
      scrolling::offset_to_reveal(current_offset, span, viewport_width)
    });

    let spans = column_spans
      .iter()
      .map(|(_, span)| *span)
      .collect::<Vec<_>>();

    let new_offset = scrolling::clamp_offset(
      revealed_offset,
      scrolling::strip_width(&spans),
      viewport_width,
    );

    self.set_scroll_offset(new_offset);
    Ok(new_offset != current_offset)
  }

  /// Effective outer gaps for this workspace.
  ///
  /// Uses `single_window_outer_gap` when the workspace has a single tiling
  /// window, otherwise falls back to `outer_gap`.
  pub fn outer_gaps(&self) -> RectDelta {
    let is_single_window = self.tiling_children().nth(1).is_none();

    let gaps_config = &self.0.borrow().gaps_config;
    let gaps = if is_single_window {
      gaps_config
        .single_window_outer_gap
        .as_ref()
        .unwrap_or(&gaps_config.outer_gap)
    } else {
      &gaps_config.outer_gap
    };

    // TODO: Should this be scaled by the monitor's DPI?
    gaps.clone()
  }

  /// Gets the bounds of a workspace with the given outer gap config.
  fn workspace_rect_with_gap_config(
    &self,
    outer_gaps: &RectDelta,
  ) -> anyhow::Result<Rect> {
    let monitor =
      self.monitor().context("Workspace has no parent monitor.")?;

    let gaps_config = &self.0.borrow().gaps_config;
    let scale_factor = if gaps_config.scale_with_dpi {
      monitor.native_properties().scale_factor
    } else {
      1.
    };

    // Get the delta between the monitor's bounds and its working area.
    let monitor_bounds = monitor.native_properties().bounds;
    let working_area_delta = monitor
      .native_properties()
      .working_area
      .delta(&monitor_bounds);

    Ok(
      monitor_bounds
        // Scale the gaps if `scale_with_dpi` is enabled. Outer gap config
        // values can be a percentage (relative to the monitor bounds), so
        // the outer gap delta needs to be applied prior to the working
        // area delta.
        .apply_delta(&outer_gaps.inverse(), Some(scale_factor))
        .apply_delta(&working_area_delta, None),
    )
  }

  /// Gets the maximum bounds of a workspace considering both `outer_gap`
  /// and `single_window_outer_gap` config values.
  pub fn max_workspace_rect(&self) -> anyhow::Result<Rect> {
    let gaps_config = &self.0.borrow().gaps_config;

    // Get the workspace rect using `outer_gap`.
    let multi_window_rect =
      self.workspace_rect_with_gap_config(&gaps_config.outer_gap)?;

    let Some(single_gap) = &gaps_config.single_window_outer_gap else {
      return Ok(multi_window_rect);
    };

    // Get the workspace rect using `single_window_outer_gap`.
    let single_window_rect =
      self.workspace_rect_with_gap_config(single_gap)?;

    Ok(multi_window_rect.union(&single_window_rect))
  }

  pub fn to_dto(&self) -> anyhow::Result<ContainerDto> {
    let rect = self.to_rect()?;
    let config = self.config();

    let children = self
      .children()
      .iter()
      .map(CommonGetters::to_dto)
      .try_collect()?;

    Ok(ContainerDto::Workspace(WorkspaceDto {
      id: self.id(),
      name: config.name,
      display_name: config.display_name,
      parent_id: self.parent().map(|parent| parent.id()),
      children,
      child_focus_order: self.0.borrow().child_focus_order.clone().into(),
      has_focus: self.has_focus(None),
      is_displayed: self.is_displayed(),
      width: rect.width(),
      height: rect.height(),
      x: rect.x(),
      y: rect.y(),
      tiling_direction: self.tiling_direction(),
      layout: self.layout(),
      scrolling: self.scrolling_state_dto()?,
    }))
  }

  /// Gets the scroll state of the workspace for IPC. Returns `None` for
  /// workspaces that don't use the scrolling layout.
  pub fn scrolling_state_dto(
    &self,
  ) -> anyhow::Result<Option<ScrollingStateDto>> {
    if !self.is_scrolling() {
      return Ok(None);
    }

    let column_spans = self.column_spans()?;
    let spans = column_spans
      .iter()
      .map(|(_, span)| *span)
      .collect::<Vec<_>>();

    Ok(Some(ScrollingStateDto {
      scroll_offset: self.scroll_offset(),
      strip_width: scrolling::strip_width(&spans),
      columns: column_spans
        .iter()
        .map(|(column, span)| ColumnDto {
          id: column.id(),
          start: span.start,
          width: span.width,
        })
        .collect(),
      focused_column_id: self.focused_column().map(|column| column.id()),
    }))
  }

  /// Records the layout and scroll state last broadcast over IPC.
  ///
  /// Returns whether the state differs from the previously recorded one,
  /// i.e. whether subscribers need to be notified.
  pub fn record_broadcast_scroll_state(
    &self,
    scroll_state: (WorkspaceLayout, Option<ScrollingStateDto>),
  ) -> bool {
    let mut inner = self.0.borrow_mut();

    if inner.broadcast_scroll_state.as_ref() == Some(&scroll_state) {
      return false;
    }

    inner.broadcast_scroll_state = Some(scroll_state);
    true
  }
}

impl_container_debug!(Workspace);
impl_common_getters!(Workspace);
impl_tiling_direction_getters!(Workspace);

impl PositionGetters for Workspace {
  fn to_rect(&self) -> anyhow::Result<Rect> {
    self.workspace_rect_with_gap_config(&self.outer_gaps())
  }
}

impl std::fmt::Display for Workspace {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(
      f,
      "Workspace(name={}, layout={:?}, tiling_direction={:?})",
      self.config().name,
      self.layout(),
      self.tiling_direction(),
    )
  }
}

#[cfg(test)]
mod tests {
  use wm_common::{TilingDirection, WorkspaceLayout};

  use crate::{
    commands::container::{
      detach_container, flatten_child_split_containers,
      set_focused_descendant, wrap_in_split_container,
    },
    models::{
      Container, Monitor, NonTilingWindow, SplitContainer,
      TilingContainer, TilingWindow, Workspace,
    },
    scrolling::ColumnVisibility,
    test_utils::{
      MOCK_MONITOR_HEIGHT, MOCK_MONITOR_WIDTH, MOCK_TASKBAR_HEIGHT,
    },
    traits::{CommonGetters, PositionGetters, TilingSizeGetters},
  };

  /// Creates a scrolling workspace on a mock monitor with the given
  /// columns. Default gaps are 0px, so the viewport is the monitor's
  /// working area.
  fn scrolling_workspace(columns: Vec<TilingContainer>) -> Workspace {
    let workspace = Workspace::mock()
      .layout(WorkspaceLayout::Scrolling)
      .tiling_containers(columns)
      .call();

    Monitor::mock().workspaces(vec![workspace.clone()]).call();
    workspace
  }

  fn window() -> TilingContainer {
    TilingWindow::mock().call().into()
  }

  #[test]
  fn scrolling_state_describes_strip_and_focus() {
    let workspace =
      scrolling_workspace(vec![window(), window(), window()]);
    let half_width = MOCK_MONITOR_WIDTH / 2;
    let third_column = workspace.tiling_children().nth(2).unwrap();
    set_focused_descendant(&third_column.clone().into(), None);
    workspace.update_scroll_offset().unwrap();

    let state = workspace.scrolling_state_dto().unwrap().unwrap();

    assert_eq!(state.scroll_offset, half_width);
    assert_eq!(state.strip_width, half_width * 3);
    assert_eq!(state.columns.len(), 3);
    assert_eq!(state.columns[2].start, half_width * 2);
    assert_eq!(state.columns[2].width, half_width);
    assert_eq!(state.focused_column_id, Some(third_column.id()));
  }

  #[test]
  fn tiling_workspaces_have_no_scrolling_state() {
    let workspace = Workspace::mock().call();
    Monitor::mock().workspaces(vec![workspace.clone()]).call();

    assert!(workspace.scrolling_state_dto().unwrap().is_none());
  }

  #[test]
  fn broadcast_state_only_reports_changes() {
    let workspace =
      scrolling_workspace(vec![window(), window(), window()]);
    let snapshot = |workspace: &Workspace| {
      (workspace.layout(), workspace.scrolling_state_dto().unwrap())
    };

    assert!(workspace.record_broadcast_scroll_state(snapshot(&workspace)));
    assert!(!workspace.record_broadcast_scroll_state(snapshot(&workspace)));

    let first_column = workspace.tiling_children().next().unwrap();
    first_column.set_tiling_size(0.67);
    assert!(workspace.record_broadcast_scroll_state(snapshot(&workspace)));
  }

  #[test]
  fn child_focus_order_yields_each_child_once() {
    let workspace =
      scrolling_workspace(vec![window(), window(), window()]);

    // Bounded with `take` so that a regression fails instead of hanging.
    assert_eq!(workspace.child_focus_order().take(10).count(), 3);
  }

  #[test]
  fn focused_column_skips_focused_non_tiling_windows() {
    let floating_window = NonTilingWindow::mock().call();
    let workspace = Workspace::mock()
      .layout(WorkspaceLayout::Scrolling)
      .tiling_containers(vec![window(), window()])
      .non_tiling_windows(vec![floating_window.clone()])
      .call();
    Monitor::mock().workspaces(vec![workspace.clone()]).call();

    let second_column = workspace.tiling_children().nth(1).unwrap();
    set_focused_descendant(&second_column.clone().into(), None);
    set_focused_descendant(&floating_window.into(), None);

    assert_eq!(
      workspace.focused_column().map(|column| column.id()),
      Some(second_column.id())
    );
  }

  #[test]
  fn columns_get_default_width_and_are_laid_out_left_to_right() {
    let workspace =
      scrolling_workspace(vec![window(), window(), window()]);
    let half_width = MOCK_MONITOR_WIDTH / 2;

    let rects = workspace
      .tiling_children()
      .map(|column| column.to_rect().unwrap())
      .collect::<Vec<_>>();

    for (index, rect) in rects.iter().enumerate() {
      #[allow(
        clippy::cast_possible_wrap,
        clippy::cast_possible_truncation
      )]
      let expected_x = half_width * index as i32;
      assert_eq!(rect.x(), expected_x);
      assert_eq!(rect.width(), half_width);
      assert_eq!(rect.height(), MOCK_MONITOR_HEIGHT - MOCK_TASKBAR_HEIGHT);
    }

    // The third column lies entirely outside the viewport.
    let third_column = workspace.tiling_children().nth(2).unwrap();
    assert_eq!(
      workspace.column_visibility(&third_column).unwrap(),
      Some(ColumnVisibility::Hidden)
    );
  }

  #[test]
  fn focusing_a_column_scrolls_minimally_to_reveal_it() {
    let workspace =
      scrolling_workspace(vec![window(), window(), window()]);
    let half_width = MOCK_MONITOR_WIDTH / 2;
    let third_column = workspace.tiling_children().nth(2).unwrap();

    set_focused_descendant(&third_column.clone().into(), None);
    assert!(workspace.update_scroll_offset().unwrap());
    assert_eq!(workspace.scroll_offset(), half_width);
    assert_eq!(third_column.to_rect().unwrap().x(), half_width);

    // Already revealed, so no further scrolling.
    assert!(!workspace.update_scroll_offset().unwrap());

    // Focusing the second column keeps the viewport where it is, since
    // the second column is already fully visible.
    let second_column = workspace.tiling_children().nth(1).unwrap();
    set_focused_descendant(&second_column.into(), None);
    assert!(!workspace.update_scroll_offset().unwrap());
    assert_eq!(workspace.scroll_offset(), half_width);
  }

  #[test]
  fn removing_a_column_keeps_sibling_widths_and_clamps_offset() {
    let workspace =
      scrolling_workspace(vec![window(), window(), window()]);
    let third_column = workspace.tiling_children().nth(2).unwrap();
    set_focused_descendant(&third_column.clone().into(), None);
    workspace.update_scroll_offset().unwrap();

    detach_container(third_column.into()).unwrap();

    for column in workspace.tiling_children() {
      assert!((column.tiling_size() - 0.5).abs() < 1e-6);
    }

    // Remaining strip exactly fills the viewport, so the offset clamps to
    // zero.
    workspace.update_scroll_offset().unwrap();
    assert_eq!(workspace.scroll_offset(), 0);
  }

  #[test]
  fn lone_column_is_resizable() {
    let workspace = scrolling_workspace(vec![window()]);
    let column = workspace.tiling_children().next().unwrap();

    let container_to_resize = column.container_to_resize(true).unwrap();
    assert_eq!(container_to_resize.map(|c| c.id()), Some(column.id()));
  }

  #[test]
  fn stacking_preserves_column_width_and_flattens_back() {
    let workspace = scrolling_workspace(vec![window(), window()]);
    let column = workspace.tiling_children().next().unwrap();
    column.set_tiling_size(0.67);

    let stack = SplitContainer::new(
      TilingDirection::Vertical,
      wm_common::GapsConfig::default(),
    );
    wrap_in_split_container(
      &stack,
      &workspace.clone().into(),
      std::slice::from_ref(&column),
    )
    .unwrap();

    assert!((stack.tiling_size() - 0.67).abs() < 1e-6);
    assert!((column.tiling_size() - 1.0).abs() < 1e-6);

    // A single-child column split is flattened, and the window keeps the
    // column's width.
    flatten_child_split_containers(&workspace.clone().into()).unwrap();
    let first_column = workspace.tiling_children().next().unwrap();
    assert_eq!(first_column.id(), column.id());
    assert!((first_column.tiling_size() - 0.67).abs() < 1e-6);
  }

  #[test]
  fn insertion_point_is_after_the_reference_column() {
    let stacked_window = window();
    let stack: TilingContainer = SplitContainer::mock()
      .tiling_direction(TilingDirection::Vertical)
      .tiling_containers(vec![window(), stacked_window.clone()])
      .call()
      .into();

    let workspace = scrolling_workspace(vec![window(), stack, window()]);

    let (parent, index) = workspace
      .tiling_insertion_point(Some(&Container::from(stacked_window)));

    assert_eq!(parent.id(), workspace.id());
    assert_eq!(index, 2);
  }
}
