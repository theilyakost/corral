use std::collections::HashMap;

use windows::Win32::Foundation::{POINT, RECT};

use crate::config::Group;
use crate::group::HEADER_HEIGHT;

const PADDING: i32 = 4;
const INSET: i32 = 6;

pub struct Icon<'a> {
    pub name: &'a str,
    pub position: POINT,
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

    pub fn content_origin(&self, group: &Group) -> POINT {
        self.snap_up(POINT { x: group.x + PADDING, y: group.y + HEADER_HEIGHT + PADDING })
    }
}

pub fn plan(icons: &[Icon], groups: &[Group], grid: Grid, work_area: impl Fn(POINT) -> RECT) -> Vec<(usize, POINT)> {
    let spacing = grid.spacing;
    let mut group_rects: Vec<RECT> = groups.iter().map(group_rect).collect();
    let mut targets: Vec<Option<POINT>> = vec![None; icons.len()];

    let mut members: Vec<Vec<usize>> = vec![Vec::new(); groups.len()];
    for (i, icon) in icons.iter().enumerate() {
        if let Some(g) = groups.iter().position(|g| g.matches(icon.name)) {
            members[g].push(i);
        }
    }
    let mut member_moves = Vec::new();
    for ((group, mut list), bounds) in groups.iter().zip(members).zip(group_rects.iter_mut()) {
        list.sort_by_key(|&i| icons[i].name.to_lowercase());
        let start = grid.content_origin(group);
        let cols = ((group.x + group.width - PADDING - start.x) / spacing.x).max(1);
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
    RECT { left: group.x, top: group.y, right: group.x + group.width, bottom: group.y + group.height }
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
        }
    }

    fn icon(name: &str, x: i32, y: i32) -> Icon<'_> {
        Icon { name, position: POINT { x, y } }
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
        let icons = [icon("b.txt", 14, 2), icon("a.txt", 14, 102), icon("c.exe", 14, 202)];
        let moves = plan(&icons, &[group(&["*.txt"])], GRID, |_| SCREEN);
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
        let icons = [icon("a.txt", 14, 2), icon("app.exe", 546, 402)];
        let g = group(&["*.txt"]);
        let moves = plan(&icons, std::slice::from_ref(&g), GRID, |_| SCREEN);
        assert_eq!(moves.len(), 2);
        assert_eq!(moves[0].0, 1);
        assert!(on_grid(moves[0].1));
        assert!(!intersects(&group_rect(&g), &cell(moves[0].1, SPACING)));
    }

    #[test]
    fn outsiders_are_pushed_out_of_overflow() {
        let mut icons: Vec<Icon> = (0..20).map(|_| icon("f.txt", 14, 2)).collect();
        icons.push(icon("app.exe", 546, 702));
        let moves = plan(&icons, &[group(&["*.txt"])], GRID, |_| SCREEN);
        assert!(moves.iter().any(|(i, _)| *i == 20));
    }

    #[test]
    fn nothing_moves_when_already_in_place() {
        let icons = [icon("app.exe", 14, 2)];
        assert!(plan(&icons, &[group(&["*.txt"])], GRID, |_| SCREEN).is_empty());
    }

    #[test]
    fn pushed_icons_do_not_stack() {
        let icons = [icon("a.exe", 546, 402), icon("b.exe", 622, 402)];
        let moves = plan(&icons, &[group(&["*.txt"])], GRID, |_| SCREEN);
        assert_eq!(moves.len(), 2);
        assert_ne!(moves[0].1, moves[1].1);
    }
}
