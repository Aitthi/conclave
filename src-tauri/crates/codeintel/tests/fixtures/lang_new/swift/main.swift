import Foundation

func greet(name: String) -> String { return "hi " + name }
func caller() { _ = greet(name: "x") }
