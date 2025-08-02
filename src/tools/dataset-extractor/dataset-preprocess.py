"""
This tool builds on the research of:
    De Keersmaeker, A. (2023). Enhancing Test Code Understandability with Machine Learning-Based Identifier Naming.
    Master’s Thesis, University of Antwerp.
"""

import os
import argparse


def extract_methods_from_dataset(tests_file, dir_out):
    """
    Splits a dataset file containing all test methods into individual files. The methods are also wrapped inside a class
    :param tests_file: huge dataset file where each line is a test method
    :param dir_out: directory where the extracted test methods will be saved
    """
    if not os.path.exists(dir_out):
        os.makedirs(dir_out)

    with open(tests_file, "r", encoding="utf-8") as file:
        counter = 1
        for line in file:
            class_name = f"TestClass{counter}"
            with open(
                f"{dir_out}/{class_name}.java", "w", encoding="utf-8"
            ) as file_out:
                # Wrap a class around the test method
                file_out.write(f"public class {class_name} {{\n")
                file_out.write(f"{line.strip()}\n")
                file_out.write("}")
            counter += 1


def main():
    parser = argparse.ArgumentParser(
        description="Extract java methods from a dataset file."
    )
    parser.add_argument(
        "-f",
        "--dfile",
        type=str,
        help="Path to the dataset file containing the methods.",
    )
    parser.add_argument(
        "-d", "--dir", type=str, help="Directory to save the extracted methods."
    )

    args = parser.parse_args()

    extract_methods_from_dataset(args.tests_file, args.dir_out)
