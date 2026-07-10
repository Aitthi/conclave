import util.Helper;

public class App {
    static int add(int a, int b) { return a + b; }

    public static void main(String[] args) {
        add(1, 2);
        Helper h = new Helper();
        h.help();
    }
}
