use schema::{TokenRecord, JobSummary, WebhookRecord};

/// Helper to print a line of dashes
fn print_divider(widths: &[usize]) {
    let mut divider = String::new();
    for &w in widths {
        divider.push('+');
        divider.push_str(&"-".repeat(w + 2));
    }
    divider.push('+');
    println!("{}", divider);
}

/// Helper to print a padded row
fn print_row(cols: &[&str], widths: &[usize]) {
    let mut row = String::new();
    for (i, &col) in cols.iter().enumerate() {
        row.push('|');
        row.push(' ');
        let val = if col.len() > widths[i] {
            let mut end = widths[i];
            while !col.is_char_boundary(end) {
                end -= 1;
            }
            &col[..end]
        } else {
            col
        };
        let padded = format!("{:width$}", val, width = widths[i]);
        row.push_str(&padded);
        row.push(' ');
    }
    row.push('|');
    println!("{}", row);
}

pub fn print_tokens(tokens: &[TokenRecord]) {
    if tokens.is_empty() {
        println!("No active tokens found.");
        return;
    }

    let headers = ["ID", "NAME", "CREATED AT"];
    let mut widths = [headers[0].len(), headers[1].len(), headers[2].len()];

    for t in tokens {
        widths[0] = widths[0].max(t.id.to_string().len());
        widths[1] = widths[1].max(t.name.len());
        widths[2] = widths[2].max(t.created_at.len());
    }

    print_divider(&widths);
    print_row(&headers, &widths);
    print_divider(&widths);

    for t in tokens {
        let id_str = t.id.to_string();
        print_row(&[&id_str, &t.name, &t.created_at], &widths);
    }
    print_divider(&widths);
}

pub fn print_jobs(jobs: &[JobSummary]) {
    if jobs.is_empty() {
        println!("No compilation jobs found.");
        return;
    }

    let headers = ["JOB ID", "PROJECT", "GIT REF", "STATUS", "QUEUED AT"];
    let mut widths = [
        headers[0].len(),
        headers[1].len(),
        headers[2].len(),
        headers[3].len(),
        headers[4].len(),
    ];

    for j in jobs {
        widths[0] = widths[0].max(j.id.len());
        widths[1] = widths[1].max(j.project.len());
        widths[2] = widths[2].max(j.git_ref.len());
        widths[3] = widths[3].max(j.status.len());
        widths[4] = widths[4].max(j.queued_at.len());
    }

    // Limit maximum column widths for terminal sanity
    if widths[1] > 60 {
        widths[1] = 60;
    }
    if widths[2] > 20 {
        widths[2] = 20;
    }

    print_divider(&widths);
    print_row(&headers, &widths);
    print_divider(&widths);

    for j in jobs {
        print_row(
            &[&j.id, &j.project, &j.git_ref, &j.status, &j.queued_at],
            &widths,
        );
    }
    print_divider(&widths);
}

pub fn print_webhooks(webhooks: &[WebhookRecord]) {
    if webhooks.is_empty() {
        println!("No webhooks registered.");
        return;
    }

    let headers = ["ID", "TARGET URL", "CREATED AT"];
    let mut widths = [headers[0].len(), headers[1].len(), headers[2].len()];

    for w in webhooks {
        widths[0] = widths[0].max(w.id.to_string().len());
        widths[1] = widths[1].max(w.url.len());
        widths[2] = widths[2].max(w.created_at.len());
    }

    // Limit URL column width to avoid extreme console stretching
    if widths[1] > 60 {
        widths[1] = 60;
    }

    print_divider(&widths);
    print_row(&headers, &widths);
    print_divider(&widths);

    for w in webhooks {
        let id_str = w.id.to_string();
        print_row(&[&id_str, &w.url, &w.created_at], &widths);
    }
    print_divider(&widths);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_print_row_multibyte_truncation_does_not_panic() {
        let widths = [2];
        let cols = ["héllo"];
        print_row(&cols, &widths);
    }
}

