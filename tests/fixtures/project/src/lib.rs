#[cfg(test)]
mod tests {
    use std::{fs, io::Write};

    #[test]
    fn reads_greeting() {
        record_execution("reads_greeting");
        let greeting = fs::read_to_string("greeting.txt").unwrap();
        assert_eq!(greeting.trim(), "hello");
    }

    #[test]
    fn reads_number() {
        record_execution("reads_number");
        let text = fs::read_to_string("number.txt").unwrap();
        let number: u32 = text.trim().parse().unwrap();
        println!("number: {number}");
        eprintln!("checking number");
        assert_eq!(number, 42);
    }

    fn record_execution(name: &str) {
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("executions")
            .unwrap()
            .write_all(format!("{name}\n").as_bytes())
            .unwrap();
    }
}
