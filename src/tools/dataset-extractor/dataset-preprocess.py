"""
This tool builds on the research of:
    De Keersmaeker, A. (2023). Enhancing Test Code Understandability with Machine Learning-Based Identifier Naming.
    Master’s Thesis, University of Antwerp.
"""

import os, shutil
import argparse

SMALL = 0.1
MEDIUM = 0.5
LARGE = 1.0


def extract_methods_from_dataset(tests_file, dir_out, size="l"):
    if not os.path.exists(dir_out):
        os.makedirs(dir_out)
        print("No directory found, diretory will be manually created")
    else:
        for filename in os.listdir(dir_out):
            print(
                f'Other files found in diretory " {dir_out} ", these will be deleted during the process of extracting methods'
            )
            file_path = os.path.join(dir_out, filename)
            try:
                if os.path.isfile(file_path) or os.path.islink(file_path):
                    os.unlink(file_path)
                elif os.path.isdir(file_path):
                    shutil.rmtree(file_path)
            except Exception as e:
                print("Failed to delete %s. Reason: %s" % (file_path, e))

    current_size = LARGE

    if size == "s":
        current_size = SMALL
    elif size == "m":
        current_size = MEDIUM

    file_size = 0

    with open(tests_file, "r", encoding="utf-8") as file:
        file_size = len(file.readlines())

    with open(tests_file, "r", encoding="utf-8") as file:
        for counter, line in enumerate(file):
            class_name = f"TestClass{counter}"
            with open(
                f"{dir_out}/{class_name}.java", "w", encoding="utf-8"
            ) as file_out:
                # Wrap a class around the test method
                file_out.write(f"public class {class_name} {{\n")
                file_out.write(f"{line.strip()}\n")
                file_out.write("}")

            if counter > current_size * file_size:
                break


def main():
    parser = argparse.ArgumentParser(
        description="Extract java methods from a dataset file."
    )
    parser.add_argument(
        "-f",
        "--file",
        type=str,
        help="Path to the dataset file containing the methods.",
    )
    parser.add_argument(
        "-d", "--dir", type=str, help="Directory to save the extracted methods."
    )
    parser.add_argument(
        "-s",
        "--size",
        type=str,
        required=False,
        default="s",
        help="Choose size with given choices: s, m, l",
        choices=["s", "m", "l"],
    )

    args = parser.parse_args()

    extract_methods_from_dataset(args.file, args.dir, args.size)


if __name__ == "__main__":
    main()
