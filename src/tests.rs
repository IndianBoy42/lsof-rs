use super::*;
// TODO: test coverage

#[test]
fn test_lsall() {
    let result = lsof().unwrap();
    println!("{:?}", result);
}

#[test]
fn test_target() {
    let filepath = "/usr/lib64/librt-2.28.so".to_owned();
    let result = lsof_file(filepath).unwrap();
    // println!("{:?}", result);
    for r in result {
        println!("pid:{}  ,name: {:?} \n", r.pid, r.name)
    }
}
