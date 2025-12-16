# Run for each directory

cargo run -- --input ../../out/dataset/test/parse_ok/ --output dataset\val\java_obfuscated\ --jsonl-output dataset\val\jsonl\

cargo run -- --input ../../out/dataset/val/parse_ok/ --output dataset\val\java_obfuscated\ --jsonl-output dataset\val\jsonl\

cargo run -- --input ../../out/dataset/train/parse_ok/ --output dataset\val\java_obfuscated\ --jsonl-output dataset\val\jsonl\
