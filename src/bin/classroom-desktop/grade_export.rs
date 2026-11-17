use classroom_rust::{
    desktop_auth::{TeacherGradebook, TeacherGradebookCell},
    desktop_data::{Coursework, SubmissionStatus},
};
use std::{io::Write, path::{Path, PathBuf}};

pub fn gradebook_csv(grid: &TeacherGradebook, columns: &[(i64, &Coursework)]) -> Vec<u8> {
    let mut csv = String::new();
    write_row(&mut csv, std::iter::once("Student").chain(columns.iter().map(|(_, item)| item.title.as_str())));
    let maximums: Vec<_> = columns.iter().map(|(_, item)| item.maximum_points.map_or_else(String::new, |points| points.to_string())).collect();
    write_row(&mut csv, std::iter::once("Points possible").chain(maximums.iter().map(String::as_str)));
    for student in &grid.students {
        let Ok(student_id) = student.id.parse::<i64>() else { continue; };
        let scores: Vec<_> = columns.iter().map(|(item, _)| grid.cells.get(&(student_id, *item))
            .and_then(exported_grade).map_or_else(String::new, |grade| grade.to_string())).collect();
        write_row(&mut csv, std::iter::once(student.display_name.as_str()).chain(scores.iter().map(String::as_str)));
    }
    csv.into_bytes()
}

fn exported_grade(cell: &TeacherGradebookCell) -> Option<f64> {
    if matches!(cell.status, SubmissionStatus::Returned | SubmissionStatus::ReturnedLate) {
        cell.released_grade.filter(|score| score.is_finite())
    } else {
        None
    }
}

fn write_row<'a>(csv: &mut String, fields: impl IntoIterator<Item = &'a str>) {
    for (index, field) in fields.into_iter().enumerate() {
        if index > 0 {
            csv.push(',');
        }
        csv.push('"');
        // Spreadsheet apps may evaluate an imported cell even if it was CSV-quoted.
        if field.starts_with(['\t', '\n', '\r'])
            || field.trim_start_matches(|ch: char| ch.is_whitespace() || ch == '\u{feff}').starts_with(['=', '+', '-', '@'])
        {
            csv.push('\'');
        }
        for ch in field.chars() {
            if ch == '"' {
                csv.push('"');
            }
            csv.push(ch);
        }
        csv.push('"');
    }
    csv.push_str("\r\n");
}

pub fn suggested_name(title: &str) -> String {
    let safe: String = title.chars().map(|ch| match ch {
        '/' | ':' | '\\' => '-',
        ch if ch.is_control() => ' ',
        ch => ch,
    }).take(80).collect();
    let safe = safe.trim_matches(|ch: char| ch.is_whitespace() || ch == '.');
    if safe.is_empty() { "Classroom grades.csv".into() }
    else { format!("{safe} grades.csv") }
}

pub async fn choose_destination(suggested: String) -> Result<Option<PathBuf>, String> {
    choose_file_destination(suggested, "Export released grades as CSV").await
}

pub async fn choose_file_destination(suggested: String, prompt: &str) -> Result<Option<PathBuf>, String> {
    let script = r#"
        on run argv
            try
                set destination to choose file name with prompt (item 2 of argv) default name (item 1 of argv) default location (path to downloads folder)
                return POSIX path of destination
            on error number -128
                return ""
            end try
        end run
    "#;
    let output = tokio::process::Command::new("/usr/bin/osascript")
        .args(["-e", script, "--", &suggested, prompt]).kill_on_drop(true).output().await
        .map_err(|_| "The Mac save dialog could not open.")?;
    if !output.status.success() {
        return Err("The Mac save dialog did not return a destination.".into());
    }
    let path = String::from_utf8(output.stdout).map_err(|_| "The save path is not valid UTF-8.")?;
    let path = path.strip_suffix('\n').unwrap_or(&path);
    Ok((!path.is_empty()).then(|| PathBuf::from(path)))
}

pub fn save_csv(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let directory = path.parent().filter(|dir| !dir.as_os_str().is_empty())
        .ok_or("Choose a folder for the grade export.")?;
    let write = || "The grade CSV could not be saved in that folder.".to_owned();
    let mut temporary = tempfile::NamedTempFile::new_in(directory).map_err(|_| write())?;
    temporary.write_all(bytes).map_err(|_| write())?;
    temporary.as_file().sync_all().map_err(|_| write())?;
    temporary.persist(path).map_err(|_| write())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use classroom_rust::desktop_data::{CourseMember, CourseRole, CourseworkKind, Snapshot};
    use std::{collections::BTreeMap, os::unix::fs::PermissionsExt};

    #[test]
    fn csv_escapes_text_and_contains_only_released_scores_in_all_columns() {
        let snapshot = Snapshot::demo();
        let mut item = snapshot.coursework_for(1).unwrap().iter().find(|item| item.kind == CourseworkKind::Assignment).unwrap().clone();
        item.title = " First, \"essay\"".into();
        item.maximum_points = Some(20.0);
        let mut second = item.clone();
        second.title = "Second".into();
        let grid = TeacherGradebook {
            students: vec![CourseMember { id: "7".into(), display_name: "=Alex".into(), role: CourseRole::Student }],
            cells: BTreeMap::from([
                ((7, 11), TeacherGradebookCell { submission_id: 100, remote_state: Some(3),
                    status: SubmissionStatus::Returned, released_grade: Some(0.0), draft_grade: Some(19.0) }),
                ((7, 12), TeacherGradebookCell { submission_id: 101, remote_state: Some(2),
                    status: SubmissionStatus::TurnedIn, released_grade: Some(5.0), draft_grade: Some(17.0) }),
            ]),
        };
        let csv = gradebook_csv(&grid, &[(11, &item), (12, &second)]);
        assert_eq!(String::from_utf8(csv).unwrap(),
            "\"Student\",\" First, \"\"essay\"\"\",\"Second\"\r\n\"Points possible\",\"20\",\"20\"\r\n\"'=Alex\",\"0\",\"\"\r\n");
    }

    #[test]
    fn grade_csv_is_saved_as_a_private_local_file() {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join(suggested_name("English / Literature"));
        save_csv(&file, b"old\r\n").unwrap();
        save_csv(&file, b"new\r\n").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"new\r\n");
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(folder.path().read_dir().unwrap().count(), 1);
    }
}
