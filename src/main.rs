use alloc_experiments::*;

fn main() {
    let _j = mem::Janitor::new(mem::AllocationContext::Arena);
    let s = String::from("Allocate and print a String in 'Arena'");
    println!("{}.", s);

    {
        let _j = mem::Janitor::new(mem::AllocationContext::Pool);
        let v = vec![123, 345, 567, 789];
        let s = format!(
            "Allocate a String and Vec and debug print it in 'Pool': {:?}",
            v
        );
        println!("{}.", s);
    }

    println!("{:?}", mem::AllocatorManager::info());
}
