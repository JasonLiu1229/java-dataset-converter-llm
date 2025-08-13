# java-dataset-converter-llm

This project is a self made java obfuscator tool, with an additional dataset formatting for LLM training. The dataset creation is optional, if the jsonl directory is not specified, it will only create the obfuscated java files.

After obfuscating the java files, these will be converted to .jsonl files, with the following layout:

```jsonl
{
  "prompt": "public class A { void b() { int x = 10; System.out.println(x); } }",
  "response": "public class Calculator { void printValue() { int value = 10; System.out.println(value); } }"
}
```

In case you make use of the method2test, there is also an another tool to help extract the java methods.
This tool builds on the research of:

```text
De Keersmaeker, A. (2023). Enhancing Test Code Understandability with Machine Learning-Based Identifier Naming.
Master’s Thesis, University of Antwerp.
```

It simply extracts the methods from the txt file and outputs it in a seperate directory. It does make use of some external methods, you can simply use poetry to install these packages if not done manually.

## Note

The obfuscation is regex based so it is not perfect.
