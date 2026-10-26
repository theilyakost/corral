use windows::Win32::Foundation::{POINT, RECT};

use crate::config::Group;
use crate::group::HEADER_HEIGHT;

const PADDING: i32 = 8;
const MARGIN: i32 = 4;
const INSET: i32 = 6;

pub struct Icon<'a> {
    pub name: &'a str,
    pub position: POINT,
}

pub fn plan(icons: &[Icon], groups: &[Group], spacing: POINT, work_area: impl Fn(POINT) -> RECT) -> Vec<(usize, POINT)> {
    let mut group_rects: Vec<RECT> = groups.iter().map(group_rect).collect();
    let mut targets: Vec<Option<POINT>> = vec![None; icons.len()];

    let mut members: Vec<Vec<usize>> = vec![Vec::new(); groups.len()];
    for (i, icon) in icons.iter().enumerate() {
        if let Some(g) = groups.iter().position(|g| g.matches(icon.name)) {
            members[g].push(i);
        }
    }
    for ((group, mut list), bounds) in groups.iter().zip(members).zip(group_rects.iter_mut()) {
        list.sort_by_key(|&i| icons[i].name.to_lowercase());
        let cols = ((group.width - 2 * PADDING) / spacing.x).max(1);
        let offset = ((group.width - cols * spacing.x) / 2).max(0);
        for (n, i) in list.into_iter().enumerate() {
            let (col, row) = (n as i32 % cols, n as i32 / cols);
            let target = POINT {
                x: group.x + offset + col * spacing.x,
                y: group.y + HEADER_HEIGHT + PADDING + row * spacing.y,
            };
            bounds.bottom = bounds.bottom.max(target.y + spacing.y);
            targets[i] = Some(target);
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
    for i in displaced {
        if let Some(free) = free_cell(work_area(icons[i].position), spacing, &group_rects, &occupied) {
            occupied.push(cell(free, spacing));
            targets[i] = Some(free);
        }
    }

    targets
        .into_iter()
        .enumerate()
        .filter_map(|(i, target)| target.filter(|t| *t != icons[i].position).map(|t| (i, t)))
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

fn free_cell(area: RECT, spacing: POINT, groups: &[RECT], occupied: &[RECT]) -> Option<POINT> {
    let mut x = area.left + MARGIN;
    while x + spacing.x <= area.right {
        let mut y = area.top + MARGIN;
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

    #[test]
    fn members_go_inside_sorted() {
        let icons = [icon("b.txt", 10, 10), icon("a.txt", 10, 110), icon("c.exe", 10, 210)];
        let moves = plan(&icons, &[group(&["*.txt"])], SPACING, |_| SCREEN);
        assert_eq!(moves.len(), 2);
        let a = moves.iter().find(|(i, _)| *i == 1).unwrap().1;
        let b = moves.iter().find(|(i, _)| *i == 0).unwrap().1;
        assert_eq!(a.y, 300 + HEADER_HEIGHT + PADDING);
        assert_eq!(b.y, a.y);
        assert_eq!(b.x, a.x + SPACING.x);
        assert!(a.x >= 500 && b.x + SPACING.x <= 900);
    }

    #[test]
    fn outsiders_are_pushed_out_of_groups() {
        let icons = [icon("app.exe", 520, 320)];
        let g = group(&["*.txt"]);
        let moves = plan(&icons, std::slice::from_ref(&g), SPACING, |_| SCREEN);
        assert_eq!(moves.len(), 1);
        let target = cell(moves[0].1, SPACING);
        assert!(!intersects(&group_rect(&g), &target));
    }

    #[test]
    fn nothing_moves_when_already_in_place() {
        let icons = [icon("app.exe", 10, 10)];
        assert!(plan(&icons, &[group(&["*.txt"])], SPACING, |_| SCREEN).is_empty());
    }

    #[test]
    fn outsiders_are_pushed_out_of_overflow() {
        let mut icons: Vec<Icon> = (0..20).map(|_| icon("f.txt", 10, 10)).collect();
        icons.push(icon("app.exe", 520, 700));
        let moves = plan(&icons, &[group(&["*.txt"])], SPACING, |_| SCREEN);
        assert!(moves.iter().any(|(i, _)| *i == 20));
    }

    #[test]
    fn pushed_icons_do_not_stack() {
        let icons = [icon("a.exe", 520, 320), icon("b.exe", 600, 320)];
        let moves = plan(&icons, &[group(&["*.txt"])], SPACING, |_| SCREEN);
        assert_eq!(moves.len(), 2);
        assert_ne!(moves[0].1, moves[1].1);
    }
}
