use std::collections::HashMap;

use windows::Win32::Foundation::{POINT, RECT};

use crate::config::Group;
use crate::group::HEADER_HEIGHT;

const PADDING: i32 = 4;
const INSET: i32 = 6;

pub struct Icon<'a> {
    pub name: &'a str,
    pub position: POINT,
    pub group: Option<usize>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Grid {
    pub origin: POINT,
    pub spacing: POINT,
}

impl Grid {
    pub fn detect(positions: impl Iterator<Item = POINT>, spacing: POINT) -> Grid {
        let mut xs: HashMap<i32, usize> = HashMap::new();
        let mut ys: HashMap<i32, usize> = HashMap::new();
        for p in positions {
            *xs.entry(p.x.rem_euclid(spacing.x)).or_default() += 1;
            *ys.entry(p.y.rem_euclid(spacing.y)).or_default() += 1;
        }
        let most_common = |m: HashMap<i32, usize>| m.into_iter().max_by_key(|&(v, n)| (n, -v)).map_or(0, |(v, _)| v);
        Grid { origin: POINT { x: most_common(xs), y: most_common(ys) }, spacing }
    }

    fn snap_up(&self, p: POINT) -> POINT {
        let up = |v: i32, o: i32, s: i32| o + (v - o + s - 1).div_euclid(s) * s;
        POINT { x: up(p.x, self.origin.x, self.spacing.x), y: up(p.y, self.origin.y, self.spacing.y) }
    }

    pub fn snap_down(&self, p: POINT) -> POINT {
        let down = |v: i32, o: i32, s: i32| o + (v - o).div_euclid(s) * s;
        POINT { x: down(p.x, self.origin.x, self.spacing.x), y: down(p.y, self.origin.y, self.spacing.y) }
    }

    pub fn content_origin(&self, group: &Group) -> POINT {
        self.snap_up(POINT { x: group.x, y: group.y + HEADER_HEIGHT + PADDING })
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Frame {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Frame {
    pub fn of(group: &Group) -> Frame {
        Frame { x: group.x, y: group.y, width: group.width, height: group.height }
    }

    pub fn apply(&self, group: &mut Group) {
        (group.x, group.y, group.width, group.height) = (self.x, self.y, self.width, self.height);
    }
}

struct Cells {
    first: POINT,
    columns: i32,
    rows: i32,
    spacing: POINT,
    area: RECT,
}

impl Cells {
    fn of(grid: Grid, area: RECT) -> Cells {
        let spacing = grid.spacing;
        let nearest = |edge: i32, origin: i32, step: i32| origin + (edge - origin + step / 2).div_euclid(step) * step;
        let first = POINT {
            x: nearest(area.left, grid.origin.x, spacing.x),
            y: nearest(area.top, grid.origin.y, spacing.y),
        };
        let count = |from: i32, to: i32, step: i32| ((to - from + step / 2).div_euclid(step)).max(1);
        let columns = count(first.x, area.right, spacing.x);
        let rows = count(first.y, area.bottom, spacing.y);
        Cells { first, columns, rows, spacing, area }
    }

    fn last(&self) -> POINT {
        POINT { x: self.first.x + self.columns * self.spacing.x, y: self.first.y + self.rows * self.spacing.y }
    }

    fn trim(&self, frame: Frame) -> Frame {
        let last = self.last();
        let left = if frame.x == self.area.left { self.first.x } else { frame.x };
        let top = if frame.y == self.area.top { self.first.y } else { frame.y };
        let right = frame.x + frame.width;
        let bottom = frame.y + frame.height;
        let right = if right == self.area.right { last.x } else { right };
        let bottom = if bottom == self.area.bottom { last.y } else { bottom };
        Frame { x: left, y: top, width: right - left, height: bottom - top }
    }

    fn span(&self, width: i32, height: i32) -> (i32, i32) {
        let n = ((width + self.spacing.x / 2) / self.spacing.x).clamp(1, self.columns);
        let m = ((height + self.spacing.y / 2) / self.spacing.y).clamp(1, self.rows);
        (n, m)
    }

    fn cell_at(&self, x: i32, y: i32) -> (i32, i32) {
        let col = (x - self.first.x + self.spacing.x / 2).div_euclid(self.spacing.x);
        let row = (y - self.first.y + self.spacing.y / 2).div_euclid(self.spacing.y);
        (col, row)
    }

    fn frame(&self, col: i32, row: i32, n: i32, m: i32) -> Frame {
        let left = if col == 0 { self.area.left } else { self.first.x + col * self.spacing.x };
        let top = if row == 0 { self.area.top } else { self.first.y + row * self.spacing.y };
        let right = if col + n >= self.columns { self.area.right } else { self.first.x + (col + n) * self.spacing.x };
        let bottom = if row + m >= self.rows { self.area.bottom } else { self.first.y + (row + m) * self.spacing.y };
        Frame { x: left, y: top, width: right - left, height: bottom - top }
    }
}

fn clamp_free(frame: Frame, area: RECT) -> Frame {
    let width = frame.width.min(area.right - area.left).max(1);
    let height = frame.height.min(area.bottom - area.top).max(1);
    let x = frame.x.clamp(area.left, area.right - width);
    let y = frame.y.clamp(area.top, area.bottom - height);
    Frame { x, y, width, height }
}

pub fn trim_group(frame: Frame, grid: Option<Grid>, area: RECT) -> Frame {
    grid.map_or(frame, |grid| Cells::of(grid, area).trim(frame))
}

pub fn fit_group(frame: Frame, grid: Option<Grid>, area: RECT) -> Frame {
    let Some(grid) = grid else { return clamp_free(frame, area) };
    let cells = Cells::of(grid, area);
    let frame = cells.trim(frame);
    let (n, m) = cells.span(frame.width, frame.height);
    let (col, row) = cells.cell_at(frame.x, frame.y);
    cells.frame(col.clamp(0, cells.columns - n), row.clamp(0, cells.rows - m), n, m)
}

pub fn resize_group(frame: Frame, grid: Option<Grid>, area: RECT) -> Frame {
    let Some(grid) = grid else {
        let width = frame.width.clamp(1, area.right - frame.x);
        let height = frame.height.clamp(HEADER_HEIGHT, area.bottom - frame.y);
        return Frame { width, height, ..frame };
    };
    let cells = Cells::of(grid, area);
    let frame = cells.trim(frame);
    let (col, row) = cells.cell_at(frame.x, frame.y);
    let (col, row) = (col.clamp(0, cells.columns - 1), row.clamp(0, cells.rows - 1));
    let (n, m) = cells.span(frame.width, frame.height);
    cells.frame(col, row, n.min(cells.columns - col), m.min(cells.rows - row))
}

pub fn nearest_free_cell(at: POINT, grid: Grid, taken: &[POINT], groups: &[Group], area: RECT) -> Option<POINT> {
    let start = grid.snap_down(at);
    let rects: Vec<RECT> = groups.iter().map(group_rect).collect();
    let (sx, sy) = (grid.spacing.x, grid.spacing.y);
    for radius in 0..16i32 {
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if dx.abs().max(dy.abs()) != radius {
                    continue;
                }
                let p = POINT { x: start.x + dx * sx, y: start.y + dy * sy };
                if p.x < area.left || p.y < area.top || p.x + sx > area.right || p.y + sy > area.bottom {
                    continue;
                }
                let candidate = cell(p, grid.spacing);
                if rects.iter().any(|r| intersects(r, &candidate)) || taken.contains(&p) {
                    continue;
                }
                return Some(p);
            }
        }
    }
    None
}

pub fn plan(icons: &[Icon], groups: &[Group], grid: Grid, work_area: impl Fn(POINT) -> RECT) -> Vec<(usize, POINT)> {
    let spacing = grid.spacing;
    let mut group_rects: Vec<RECT> = groups.iter().map(group_rect).collect();
    let mut targets: Vec<Option<POINT>> = vec![None; icons.len()];

    let mut members: Vec<Vec<usize>> = vec![Vec::new(); groups.len()];
    for (i, icon) in icons.iter().enumerate() {
        if let Some(list) = icon.group.and_then(|g| members.get_mut(g)) {
            list.push(i);
        }
    }
    let mut member_moves = Vec::new();
    for ((group, mut list), bounds) in groups.iter().zip(members).zip(group_rects.iter_mut()) {
        list.sort_by_key(|&i| icons[i].name.to_lowercase());
        if group.collapsed {
            let stack = grid.snap_down(POINT { x: group.x, y: group.y });
            if !list.is_empty() {
                let stack_cell = cell(stack, spacing);
                bounds.left = bounds.left.min(stack_cell.left);
                bounds.bottom = bounds.bottom.max(stack_cell.bottom);
            }
            for i in list {
                targets[i] = Some(stack);
                member_moves.push(i);
            }
            continue;
        }
        let start = grid.content_origin(group);
        let cols = ((group.x + group.width - start.x) / spacing.x).max(1);
        for (n, i) in list.into_iter().enumerate() {
            let (col, row) = (n as i32 % cols, n as i32 / cols);
            let target = POINT { x: start.x + col * spacing.x, y: start.y + row * spacing.y };
            bounds.bottom = bounds.bottom.max(target.y + spacing.y);
            targets[i] = Some(target);
            member_moves.push(i);
        }
    }

    let mut occupied = Vec::new();
    let mut displaced = Vec::new();
    for (i, icon) in icons.iter().enumerate() {
        match targets[i] {
            Some(target) => occupied.push(cell(target, spacing)),
            None if group_rects.iter().any(|g| intersects(g, &cell(icon.position, spacing))) => displaced.push(i),
            None => occupied.push(cell(icon.position, spacing)),
        }
    }
    for &i in &displaced {
        if let Some(free) = free_cell(work_area(icons[i].position), grid, &group_rects, &occupied) {
            occupied.push(cell(free, spacing));
            targets[i] = Some(free);
        }
    }

    displaced
        .into_iter()
        .chain(member_moves)
        .filter_map(|i| targets[i].filter(|t| *t != icons[i].position).map(|t| (i, t)))
        .collect()
}

fn group_rect(group: &Group) -> RECT {
    let height = if group.collapsed { HEADER_HEIGHT } else { group.height };
    RECT { left: group.x, top: group.y, right: group.x + group.width, bottom: group.y + height }
}

fn cell(position: POINT, spacing: POINT) -> RECT {
    RECT {
        left: position.x + INSET,
        top: position.y + INSET,
        right: position.x + spacing.x - INSET,
        bottom: position.y + spacing.y - INSET,
    }
}

fn intersects(a: &RECT, b: &RECT) -> bool {
    a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom
}

fn free_cell(area: RECT, grid: Grid, groups: &[RECT], occupied: &[RECT]) -> Option<POINT> {
    let spacing = grid.spacing;
    let start = grid.snap_up(POINT { x: area.left, y: area.top });
    let mut x = start.x;
    while x + spacing.x <= area.right {
        let mut y = start.y;
        while y + spacing.y <= area.bottom {
            let candidate = cell(POINT { x, y }, spacing);
            if !groups.iter().chain(occupied).any(|r| intersects(r, &candidate)) {
                return Some(POINT { x, y });
            }
            y += spacing.y;
        }
        x += spacing.x;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::group_of;

    const SPACING: POINT = POINT { x: 76, y: 100 };
    const GRID: Grid = Grid { origin: POINT { x: 14, y: 2 }, spacing: SPACING };
    const SCREEN: RECT = RECT { left: 0, top: 0, right: 1920, bottom: 1040 };

    fn group(rules: &[&str]) -> Group {
        Group {
            name: "g".into(),
            x: 500,
            y: 300,
            width: 400,
            height: 300,
            rules: rules.iter().map(|r| r.to_string()).collect(),
            ..Group::empty()
        }
    }

    fn icon(name: &str, x: i32, y: i32) -> Icon<'_> {
        Icon { name, position: POINT { x, y }, group: None }
    }

    fn run(mut icons: Vec<Icon>, groups: &[Group]) -> Vec<(usize, POINT)> {
        for icon in &mut icons {
            icon.group = group_of(groups, icon.name);
        }
        plan(&icons, groups, GRID, |_| SCREEN)
    }

    fn on_grid(p: POINT) -> bool {
        (p.x - GRID.origin.x).rem_euclid(SPACING.x) == 0 && (p.y - GRID.origin.y).rem_euclid(SPACING.y) == 0
    }

    #[test]
    fn detects_grid_origin() {
        let positions = [POINT { x: 14, y: 2 }, POINT { x: 90, y: 102 }, POINT { x: 1306, y: 602 }, POINT { x: 500, y: 7 }];
        assert_eq!(Grid::detect(positions.into_iter(), SPACING), GRID);
    }

    #[test]
    fn members_go_inside_sorted_on_grid() {
        let icons = vec![icon("b.txt", 14, 2), icon("a.txt", 14, 102), icon("c.exe", 14, 202)];
        let moves = run(icons, &[group(&["*.txt"])]);
        assert_eq!(moves.len(), 2);
        let a = moves.iter().find(|(i, _)| *i == 1).unwrap().1;
        let b = moves.iter().find(|(i, _)| *i == 0).unwrap().1;
        assert!(on_grid(a) && on_grid(b));
        assert_eq!(b.y, a.y);
        assert_eq!(b.x, a.x + SPACING.x);
        assert!(a.x >= 500 && a.y >= 300 + HEADER_HEIGHT && b.x + SPACING.x <= 900);
    }

    #[test]
    fn outsiders_are_pushed_out_first() {
        let icons = vec![icon("a.txt", 14, 2), icon("app.exe", 546, 402)];
        let g = group(&["*.txt"]);
        let moves = run(icons, std::slice::from_ref(&g));
        assert_eq!(moves.len(), 2);
        assert_eq!(moves[0].0, 1);
        assert!(on_grid(moves[0].1));
        assert!(!intersects(&group_rect(&g), &cell(moves[0].1, SPACING)));
    }

    #[test]
    fn outsiders_are_pushed_out_of_overflow() {
        let mut icons: Vec<Icon> = (0..20).map(|_| icon("f.txt", 14, 2)).collect();
        icons.push(icon("app.exe", 546, 702));
        let moves = run(icons, &[group(&["*.txt"])]);
        assert!(moves.iter().any(|(i, _)| *i == 20));
    }

    #[test]
    fn nothing_moves_when_already_in_place() {
        assert!(run(vec![icon("app.exe", 14, 2)], &[group(&["*.txt"])]).is_empty());
    }

    #[test]
    fn manual_membership_wins_over_rules() {
        let mut docs = group(&["*.txt"]);
        docs.exclude.push("b.txt".into());
        let mut apps = Group { x: 1000, ..group(&[]) };
        apps.items.push("a.txt".into());
        let moves = run(vec![icon("a.txt", 14, 2), icon("b.txt", 14, 102)], &[docs, apps]);
        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].0, 0);
        assert!(moves[0].1.x >= 1000);
    }

    #[test]
    fn collapsed_group_reserves_only_its_header() {
        let mut g = group(&["*.txt"]);
        g.collapsed = true;
        let icons = vec![icon("a.txt", 14, 2), icon("b.txt", 14, 102), icon("app.exe", 622, 502)];
        let moves = run(icons, std::slice::from_ref(&g));
        assert_eq!(moves.len(), 2);
        assert!(moves.iter().all(|(i, _)| *i < 2));
        assert_eq!(moves[0].1, moves[1].1);
        assert!(moves[0].1.y <= 300 && moves[0].1.y + SPACING.y > 300);
    }

    fn frame(x: i32, y: i32, width: i32, height: i32) -> Frame {
        Frame { x, y, width, height }
    }

    fn on_cells(f: Frame) -> bool {
        on_grid(POINT { x: f.x, y: f.y }) && f.width % SPACING.x == 0 && f.height % SPACING.y == 0
    }

    #[test]
    fn fitted_group_covers_whole_cells() {
        let f = fit_group(frame(523, 291, 420, 260), Some(GRID), SCREEN);
        assert!(on_cells(f), "{f:?}");
        assert_eq!((f.width, f.height), (6 * SPACING.x, 3 * SPACING.y));
        assert!((f.x - 523).abs() <= SPACING.x / 2 && (f.y - 291).abs() <= SPACING.y / 2);
        let g = Group { x: f.x, y: f.y, ..group(&[]) };
        assert_eq!(GRID.content_origin(&g).x, f.x);
    }

    #[test]
    fn fitted_group_reaches_the_screen_edges() {
        let f = fit_group(frame(5000, 5000, 400, 300), Some(GRID), SCREEN);
        assert_eq!((f.x + f.width, f.y + f.height), (SCREEN.right, SCREEN.bottom));
        assert!(on_grid(POINT { x: f.x, y: f.y }));
        let f = fit_group(frame(-500, -500, 400, 300), Some(GRID), SCREEN);
        assert_eq!((f.x, f.y), (SCREEN.left, SCREEN.top));
        let inner = GRID.content_origin(&Group { x: f.x, y: f.y, ..group(&[]) });
        assert!(on_grid(inner) && inner.x == GRID.origin.x);
        let f = fit_group(frame(100, 100, 40000, 300), Some(GRID), SCREEN);
        assert_eq!((f.x, f.x + f.width), (SCREEN.left, SCREEN.right));
        let f = fit_group(frame(-50, 2000, 400, 300), None, SCREEN);
        assert_eq!((f.x, f.y), (0, 740));
    }

    #[test]
    fn edge_groups_keep_their_size_when_moved_inward() {
        let edge = fit_group(frame(0, 0, 4 * SPACING.x, 3 * SPACING.y), Some(GRID), SCREEN);
        assert_eq!((edge.x, edge.y), (0, 0));
        let start = trim_group(edge, Some(GRID), SCREEN);
        assert!(on_cells(start) && start.width == 4 * SPACING.x && start.height == 3 * SPACING.y);
        let moved = fit_group(Frame { x: start.x + 300, y: start.y + 300, ..start }, Some(GRID), SCREEN);
        assert!(on_cells(moved));
        assert_eq!((moved.width, moved.height), (4 * SPACING.x, 3 * SPACING.y));
        assert_eq!(fit_group(edge, Some(GRID), SCREEN), edge);
    }

    #[test]
    fn edge_cells_never_add_or_lose_a_column() {
        let shifted = Grid { origin: POINT { x: 70, y: 4 }, spacing: SPACING };
        for (x, y) in [(0, 0), (300, 300), (5000, 5000), (-50, 900)] {
            let f = fit_group(frame(x, y, 3 * SPACING.x, 3 * SPACING.y), Some(shifted), SCREEN);
            assert!((f.width - 3 * SPACING.x).abs() < SPACING.x / 2, "{x},{y} -> {f:?}");
            assert!((f.height - 3 * SPACING.y).abs() < SPACING.y / 2, "{x},{y} -> {f:?}");
            assert!(f.x >= SCREEN.left && f.x + f.width <= SCREEN.right);
            assert!(f.y >= SCREEN.top && f.y + f.height <= SCREEN.bottom);
        }
        let edge = fit_group(frame(0, 0, 3 * SPACING.x, 3 * SPACING.y), Some(shifted), SCREEN);
        assert_eq!((edge.x, edge.y), (0, 0));
        let start = trim_group(edge, Some(shifted), SCREEN);
        let moved = fit_group(Frame { x: start.x + 400, y: start.y + 400, ..start }, Some(shifted), SCREEN);
        assert_eq!((moved.width, moved.height), (3 * SPACING.x, 3 * SPACING.y));
        assert_eq!((moved.x - shifted.origin.x).rem_euclid(SPACING.x), 0);
    }

    #[test]
    fn resize_keeps_the_corner_and_steps_by_cells() {
        let start = fit_group(frame(500, 300, 400, 300), Some(GRID), SCREEN);
        let wider = resize_group(Frame { width: start.width + 50, ..start }, Some(GRID), SCREEN);
        assert_eq!((wider.x, wider.y, wider.width), (start.x, start.y, start.width + SPACING.x));
        let tiny = resize_group(Frame { width: 3, height: -40, ..start }, Some(GRID), SCREEN);
        assert_eq!((tiny.x, tiny.y, tiny.width, tiny.height), (start.x, start.y, SPACING.x, SPACING.y));
        let huge = resize_group(Frame { width: 9000, height: 9000, ..start }, Some(GRID), SCREEN);
        assert_eq!((huge.x + huge.width, huge.y + huge.height), (SCREEN.right, SCREEN.bottom));
        assert_eq!((huge.x, huge.y), (start.x, start.y));
    }

    #[test]
    fn dropped_icon_lands_in_the_cell_under_the_cursor_or_next_to_it() {
        let g = group(&[]);
        let free = nearest_free_cell(POINT { x: 130, y: 150 }, GRID, &[], std::slice::from_ref(&g), SCREEN);
        assert_eq!(free, Some(POINT { x: 90, y: 102 }));
        let free = nearest_free_cell(POINT { x: 130, y: 150 }, GRID, &[POINT { x: 90, y: 102 }], &[], SCREEN).unwrap();
        assert!(on_grid(free) && free != POINT { x: 90, y: 102 });
        assert!((free.x - 90).abs() <= SPACING.x && (free.y - 102).abs() <= SPACING.y);
        let inside = nearest_free_cell(POINT { x: 600, y: 400 }, GRID, &[], std::slice::from_ref(&g), SCREEN).unwrap();
        assert!(!intersects(&group_rect(&g), &cell(inside, SPACING)));
    }

    #[test]
    fn pushed_icons_do_not_stack() {
        let icons = vec![icon("a.exe", 546, 402), icon("b.exe", 622, 402)];
        let moves = run(icons, &[group(&["*.txt"])]);
        assert_eq!(moves.len(), 2);
        assert_ne!(moves[0].1, moves[1].1);
    }
}
