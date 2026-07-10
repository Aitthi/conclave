class Shape { public: int area(); };
int Shape::area() { return 0; }
int use() { Shape s; return s.area(); }
