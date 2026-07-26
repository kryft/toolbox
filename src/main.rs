fn main() {
    println!("Hello, world!");
}

#[cfg(test)]
mod tests {
    #[test]
    fn basic_test_passes() {
        assert!(true);
    }
}
